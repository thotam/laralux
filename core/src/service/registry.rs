use crate::config::Config;
use crate::paths::LaragonPaths;
use crate::service::mailpit::MailpitService;
use crate::service::mariadb::MariadbService;
use crate::service::nginx::NginxService;
use crate::service::php_fpm::PhpFpmService;
use crate::service::redis::RedisService;
use crate::service::Service;

/// Build the set of services enabled in `config`, wiring nginx to the
/// php-fpm socket for the configured PHP version.
pub fn build_services(config: &Config, paths: &LaragonPaths) -> Vec<Box<dyn Service>> {
    let mut services: Vec<Box<dyn Service>> = Vec::new();
    let php = PhpFpmService::new(config.php_version.clone());
    let php_socket = php.socket_path(paths);

    if config.services.mariadb {
        services.push(Box::new(MariadbService::new()));
    }
    if config.services.redis {
        services.push(Box::new(RedisService::new()));
    }
    if config.services.php {
        services.push(Box::new(php));
    }
    if config.services.nginx {
        services.push(Box::new(NginxService::new(php_socket)));
    }
    if config.services.mailpit {
        services.push(Box::new(MailpitService::new()));
    }
    services
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::paths::LaragonPaths;
    use crate::service::ServiceKind;

    #[test]
    fn builds_all_enabled_services() {
        let cfg = Config::default();
        let p = LaragonPaths::new("/tmp/lara".into());
        let svcs = build_services(&cfg, &p);
        let kinds: Vec<ServiceKind> = svcs.iter().map(|s| s.kind()).collect();
        for k in [
            ServiceKind::Nginx,
            ServiceKind::PhpFpm,
            ServiceKind::Mariadb,
            ServiceKind::Redis,
            ServiceKind::Mailpit,
        ] {
            assert!(kinds.contains(&k), "missing {k:?}");
        }
    }

    #[test]
    fn omits_disabled_services() {
        let mut cfg = Config::default();
        cfg.services.redis = false;
        cfg.services.mailpit = false;
        let p = LaragonPaths::new("/tmp/lara".into());
        let kinds: Vec<ServiceKind> =
            build_services(&cfg, &p).iter().map(|s| s.kind()).collect();
        assert!(!kinds.contains(&ServiceKind::Redis));
        assert!(!kinds.contains(&ServiceKind::Mailpit));
        assert!(kinds.contains(&ServiceKind::Nginx));
    }
}
