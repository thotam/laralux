use crate::paths::LaraluxPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};

pub struct MailpitService {
    smtp_port: u16,
    ui_port: u16,
}

impl MailpitService {
    pub fn new() -> Self {
        Self { smtp_port: 1025, ui_port: 8025 }
    }
}

impl Default for MailpitService {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for MailpitService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Mailpit
    }
    fn name(&self) -> &str {
        "mailpit"
    }
    fn command(&self, _paths: &LaraluxPaths) -> SpawnSpec {
        SpawnSpec::new("mailpit")
            .arg("--listen")
            .arg(format!("127.0.0.1:{}", self.ui_port))
            .arg("--smtp")
            .arg(format!("127.0.0.1:{}", self.smtp_port))
    }
    fn health_check(&self, _paths: &LaraluxPaths) -> Result<(), ServiceError> {
        probe_tcp(self.ui_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaraluxPaths;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_sets_listen_and_smtp_flags() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        let svc = MailpitService::new();
        let spec = svc.command(&p);
        assert_eq!(spec.program, "mailpit");
        let joined = spec.args.join(" ");
        assert!(joined.contains("--listen"));
        assert!(joined.contains("8025"));
        assert!(joined.contains("--smtp"));
        assert!(joined.contains("1025"));
        assert_eq!(svc.kind(), ServiceKind::Mailpit);
    }
}
