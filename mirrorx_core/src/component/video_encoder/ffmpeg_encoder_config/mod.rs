mod h264_videotoolbox;
mod hevc_videotoolbox;
mod libx264;

use crate::{
    core_error,
    error::{CoreError, CoreResult},
    ffi::ffmpeg::{
        avcodec::{AVCodecContext, AVCodecID},
        avutil::{av_opt_set, AVERROR, AVERROR_OPTION_NOT_FOUND},
    },
};
use std::ffi::CString;

pub enum FFMPEGEncoderType {
    Libx264,
    H264VideoToolbox,
    HEVCVideoToolbox,
}

impl FFMPEGEncoderType {
    pub fn create_config(self) -> Box<dyn FFMPEGEncoderConfig> {
        match self {
            FFMPEGEncoderType::Libx264 => Box::new(libx264::Libx264Config::new()),
            FFMPEGEncoderType::H264VideoToolbox => {
                Box::new(h264_videotoolbox::H264VideoToolboxConfig::new())
            }
            FFMPEGEncoderType::HEVCVideoToolbox => {
                Box::new(hevc_videotoolbox::HEVCVideoToolboxConfig::new())
            }
        }
    }
}

pub trait FFMPEGEncoderConfig {
    fn apply_option(&self, codec_ctx: *mut AVCodecContext) -> CoreResult<()>;
    fn ffmpeg_encoder_name(&self) -> *const i8;
    fn av_codec_id(&self) -> AVCodecID;
}

fn set_codec_ctx_option(
    codec_ctx: *mut AVCodecContext,
    key: &str,
    value: &str,
    search_flags: i32,
) -> CoreResult<()> {
    let opt_name = CString::new(key.to_string())?;
    let opt_value = CString::new(value.to_string())?;

    unsafe {
        let ret = av_opt_set(
            (*codec_ctx).priv_data,
            opt_name.as_ptr(),
            opt_value.as_ptr(),
            search_flags,
        );

        if ret == AVERROR_OPTION_NOT_FOUND {
            return Err(core_error!(
                "set AVCodecContext returns AVERROR_OPTION_NOT_FOUND"
            ));
        } else if ret == AVERROR(libc::ERANGE) {
            return Err(core_error!("set AVCodecContext returns ERANGE"));
        } else if ret == AVERROR(libc::EINVAL) {
            return Err(core_error!("set AVCodecContext returns EINVAL"));
        } else if ret != 0 {
            return Err(core_error!(
                "set AVCodecContext returns error code: {}",
                ret
            ));
        } else {
            Ok(())
        }
    }
}
