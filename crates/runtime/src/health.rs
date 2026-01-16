use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthModules {
    pub search: ModuleStatus,
    pub database: DatabaseStatus,
    pub comments: ModuleStatus,
    pub kudos: KudosStatus,
    pub pulse: PulseStatus,
    pub douban: ModuleStatus,
    pub webhook: WebhookStatus,
}

#[derive(Debug, Serialize)]
pub struct ModuleStatus {
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct DatabaseStatus {
    pub configured: bool,
}

#[derive(Debug, Serialize)]
pub struct KudosStatus {
    pub enabled: bool,
    pub cookie_ready: bool,
    pub token_ready: bool,
}

#[derive(Debug, Serialize)]
pub struct PulseStatus {
    pub enabled: bool,
    pub cookie_ready: bool,
    pub token_ready: bool,
}

#[derive(Debug, Serialize)]
pub struct WebhookStatus {
    pub configured: bool,
}
