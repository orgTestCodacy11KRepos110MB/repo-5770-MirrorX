use crate::{
    api::endpoint::message::EndPointAudioFrame,
    core_error,
    error::{CoreError, CoreResult},
    ffi::opus::decoder::{
        opus_decode_float, opus_decoder_create, opus_decoder_destroy, OpusDecoder,
    },
};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BufferSize, OutputCallbackInfo, SampleRate,
};
use tokio::sync::mpsc::{
    error::{TryRecvError, TrySendError},
    Receiver, Sender,
};

pub struct AudioPlayer {
    sample_rate: u32,
    channels: u8,
    frame_size: u16,
    decode_context: Option<DecodeContext>,
    playback_context: Option<PlaybackContext>,
}

unsafe impl Send for AudioPlayer {}

impl AudioPlayer {
    pub fn new() -> Self {
        AudioPlayer {
            sample_rate: 0,
            channels: 0,
            frame_size: 0,
            decode_context: None,
            playback_context: None,
        }
    }

    pub fn play_samples(&mut self, audio_frame: EndPointAudioFrame) -> CoreResult<()> {
        if let Some((sample_rate, sample_format, channels, frame_size)) = audio_frame.params {
            tracing::info!(
                "sample_rate: {}, channels: {}, frame_size:{}",
                sample_rate,
                channels,
                frame_size
            );

            let decode_context = DecodeContext::new(sample_rate, channels, frame_size)?;
            let playback_context =
                PlaybackContext::new(sample_rate, channels, frame_size as usize)?;

            self.sample_rate = sample_rate;
            self.channels = channels;
            self.frame_size = frame_size;
            self.decode_context = Some(decode_context);
            self.playback_context = Some(playback_context);
        }

        let decoded_buffer = if let Some(decode_context) = self.decode_context.as_mut() {
            decode_context.decode(&audio_frame.buffer)
        } else {
            Err(core_error!("audio player decode context not initialized"))
        }?;

        let success = if let Some(playback_context) = self.playback_context.as_mut() {
            Ok(playback_context.enqueue_samples(decoded_buffer))
        } else {
            Err(core_error!("audio player playback context not initialized"))
        }?;

        if !success {
            let mut playback_context =
                PlaybackContext::new(self.sample_rate, self.channels, self.frame_size as usize)?;

            if !playback_context.enqueue_samples(decoded_buffer) {
                return Err(core_error!("too many playback context initialize failures"));
            }

            self.playback_context = Some(playback_context);
        }

        Ok(())
    }
}

struct DecodeContext {
    dec: *mut OpusDecoder,
    dec_buffer: Vec<f32>,
    channels: u8,
    frame_size: u16,
}

impl DecodeContext {
    pub fn new(sample_rate: u32, channels: u8, frame_size: u16) -> CoreResult<Self> {
        unsafe {
            let mut error_code = 0;
            let dec = opus_decoder_create(sample_rate as i32, channels as isize, &mut error_code);

            if dec.is_null() {
                return Err(core_error!("opus_decoder_create returns null"));
            }

            if error_code != 0 {
                return Err(core_error!(
                    "opus_decoder_create returns error code: {}",
                    error_code
                ));
            }

            let buffer_size = frame_size * (channels as u16);
            let mut dec_buffer = Vec::new();
            dec_buffer.resize(buffer_size as usize, 0f32);

            Ok(Self {
                dec,
                dec_buffer,
                channels,
                frame_size,
            })
        }
    }

    pub fn decode(&mut self, buffer: &[u8]) -> CoreResult<&[f32]> {
        unsafe {
            let ret = opus_decode_float(
                self.dec,
                buffer.as_ptr(),
                buffer.len() as i32,
                self.dec_buffer.as_mut_ptr(),
                (self.frame_size) as isize,
                0,
            );

            if ret < 0 {
                return Err(core_error!("opus_decode_float returns error code: {}", ret));
            }

            Ok(&self.dec_buffer[0..((ret as usize) * (self.channels as usize))])
        }
    }
}

impl Drop for DecodeContext {
    fn drop(&mut self) {
        if !self.dec.is_null() {
            unsafe {
                opus_decoder_destroy(self.dec);
            }
        }
    }
}

struct PlaybackContext {
    stream: cpal::Stream,
    audio_sample_tx: Sender<Vec<f32>>,
    callback_exit_rx: Receiver<()>,
}

impl PlaybackContext {
    pub fn new(sample_rate: u32, channels: u8, frame_size: usize) -> CoreResult<Self> {
        let host = cpal::default_host();

        let device = match host.default_output_device() {
            Some(device) => device,
            None => {
                return Err(core_error!("default audio output device not exist"));
            }
        };

        tracing::info!(name = ?device.name(), "select audio output device");

        let stream_config = cpal::StreamConfig {
            channels: channels as u16,
            sample_rate: SampleRate(sample_rate),
            // actual buffer_size will be frame_size * channels, and stream config has specified channels so
            // here we just give it frame_size
            buffer_size: BufferSize::Fixed(frame_size as u32),
        };
        tracing::info!(?stream_config, "select audio stream config");

        let (audio_sample_tx, mut audio_sample_rx) = tokio::sync::mpsc::channel::<Vec<f32>>(64);
        let (callback_exit_tx, callback_exit_rx) = tokio::sync::mpsc::channel(1);
        let err_callback_exit_tx = callback_exit_tx.clone();

        let input_callback = move |data: &mut [f32], info: &OutputCallbackInfo| {
            if let Ok(samples) = audio_sample_rx.try_recv() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        samples.as_ptr(),
                        data.as_mut_ptr(),
                        samples.len().min(data.len()),
                    )
                }
            }
            // match audio_sample_rx.read_chunk(data.len().min(audio_sample_rx.slots())) {
            //     Ok(chunk) => {
            //         let (c1, c2) = chunk.as_slices();
            //         let c1_copy_length = c1.len().min(data.len());
            //         unsafe {
            //             std::ptr::copy_nonoverlapping(
            //                 c1.as_ptr(),
            //                 data.as_mut_ptr(),
            //                 c1_copy_length,
            //             );
            //         }

            //         let remaining = data.len() - c1_copy_length;
            //         let mut c2_copy_length = 0;
            //         if remaining > 0 && !c2.is_empty() {
            //             c2_copy_length = remaining.min(c2.len());
            //             unsafe {
            //                 std::ptr::copy_nonoverlapping(
            //                     c2.as_ptr(),
            //                     data[c1_copy_length..].as_mut_ptr(),
            //                     c2_copy_length,
            //                 );
            //             }
            //         }

            //         chunk.commit(c1_copy_length + c2_copy_length)
            //     }
            //     Err(err) => {
            //         tracing::error!(?err, "audio sample rx required invalid slots capacity");
            //         let _ = callback_exit_tx.send(());
            //     }
            // };
        };

        let err_callback = move |err| {
            tracing::error!(?err, "error occurred on the output audio stream");
            let _ = err_callback_exit_tx.send(());
        };

        let stream = device.build_output_stream(&stream_config, input_callback, err_callback)?;
        stream.play()?;

        Ok(PlaybackContext {
            stream,
            audio_sample_tx,
            callback_exit_rx,
        })
    }

    pub fn enqueue_samples(&mut self, mut buffer: &[f32]) -> bool {
        match self.callback_exit_rx.try_recv() {
            Ok(_) => return false,
            Err(err) => {
                if err == TryRecvError::Disconnected {
                    return false;
                }
            }
        };

        if let Err(TrySendError::Closed(_)) = self.audio_sample_tx.try_send(buffer.to_vec()) {
            return false;
        }

        // while !(buffer).is_empty() {
        //     match self
        //         .audio_sample_tx
        //         .write_chunk_uninit(buffer.len().min(self.audio_sample_tx.slots()))
        //     {
        //         Ok(mut chunk) => {
        //             let (c1, c2) = chunk.as_mut_slices();

        //             let mut it = buffer.iter();
        //             let mut iterated = 0;
        //             'outer: for &(ptr, len) in &[
        //                 (c1.as_mut_ptr() as *mut f32, c1.len()),
        //                 (c2.as_mut_ptr() as *mut f32, c2.len()),
        //             ] {
        //                 for i in 0..len {
        //                     match it.next() {
        //                         Some(item) => {
        //                             unsafe {
        //                                 ptr.add(i).write(*item);
        //                             }
        //                             iterated += 1;
        //                         }
        //                         None => break 'outer,
        //                     }
        //                 }
        //             }

        //             unsafe { chunk.commit(iterated) }
        //             buffer = &buffer[iterated..];
        //         }
        //         Err(err) => {
        //             tracing::error!(?err, "audio sample tx required invalid slots capacity");
        //             return false;
        //         }
        //     }
        // }

        true
    }

    pub fn pause(&self) {
        let _ = self.stream.pause();
    }
}

impl Drop for PlaybackContext {
    fn drop(&mut self) {
        self.pause()
    }
}