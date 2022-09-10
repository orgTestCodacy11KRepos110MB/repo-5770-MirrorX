use super::SignalingClientManager;
use crate::error::CoreResult;

pub enum ResourceType {
    Desktop,
    Files,
}

pub struct VisitRequest {
    pub local_device_id: String,
    pub remote_device_id: String,
    pub resource_type: ResourceType,
}

pub struct VisitResponse {
    pub allow: bool,
}

pub async fn visit(req: VisitRequest) -> CoreResult<VisitResponse> {
    let resource_type = match req.resource_type {
        ResourceType::Desktop => crate::proto::signaling::ResourceType::Desktop,
        ResourceType::Files => crate::proto::signaling::ResourceType::Files,
    };

    let resp = SignalingClientManager::get_client()
        .await?
        .visit(crate::proto::signaling::VisitRequest {
            active_device_id: req.local_device_id,
            passive_device_id: req.remote_device_id,
            resource_type: resource_type.into(),
        })
        .await?
        .into_inner();

    Ok(VisitResponse { allow: resp.allow })
}
