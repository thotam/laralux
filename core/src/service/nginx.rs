use crate::paths::LaraluxPaths;
use crate::service::{probe_tcp, Service, ServiceError, ServiceKind, SpawnSpec};
use std::path::PathBuf;

pub struct NginxService {
    http_port: u16,
    php_socket: PathBuf,
}

impl NginxService {
    pub fn new(php_socket: PathBuf) -> Self {
        Self { http_port: 80, php_socket }
    }
    fn conf_path(&self, paths: &LaraluxPaths) -> PathBuf {
        paths.etc_for("nginx").join("nginx.conf")
    }
}

impl Service for NginxService {
    fn kind(&self) -> ServiceKind {
        ServiceKind::Nginx
    }
    fn name(&self) -> &str {
        "nginx"
    }
    fn deps(&self) -> &[ServiceKind] {
        const DEPS: [ServiceKind; 1] = [ServiceKind::PhpFpm];
        &DEPS
    }
    fn write_config(&self, paths: &LaraluxPaths) -> Result<(), ServiceError> {
        std::fs::create_dir_all(paths.etc_for("nginx").join("sites"))?;
        std::fs::create_dir_all(paths.tmp())?;
        std::fs::create_dir_all(paths.log())?;
        let conf = format!(
            "worker_processes auto;\n\
             pid {pid};\n\
             error_log {errlog};\n\
             events {{ worker_connections 1024; }}\n\
             http {{\n\
             \x20 map $http_upgrade $connection_upgrade {{ default upgrade; '' close; }}\n\
             \x20 access_log {acclog};\n\
             \x20 client_body_temp_path {tmp}/nginx-client;\n\
             \x20 proxy_temp_path {tmp}/nginx-proxy;\n\
             \x20 fastcgi_temp_path {tmp}/nginx-fastcgi;\n\
             \x20 default_type application/octet-stream;\n\
             \x20 server {{\n\
             \x20   listen {port};\n\
             \x20   server_name localhost;\n\
             \x20   root {www};\n\
             \x20   index index.php index.html;\n\
             \x20   location / {{ try_files $uri $uri/ /index.php?$query_string; }}\n\
             \x20   location ~ \\.php$ {{\n\
             \x20     fastcgi_pass unix:{sock};\n\
             \x20     fastcgi_index index.php;\n\
             \x20     include {nginx_etc}/fastcgi_params;\n\
             \x20     fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n\
             \x20   }}\n\
             \x20 }}\n\
             \x20 include {nginx_etc}/sites/*.conf;\n\
             }}\n",
            pid = paths.tmp().join("nginx.pid").display(),
            errlog = paths.log().join("nginx-error.log").display(),
            acclog = paths.log().join("nginx-access.log").display(),
            tmp = paths.tmp().display(),
            port = self.http_port,
            www = paths.www().display(),
            sock = self.php_socket.display(),
            nginx_etc = paths.etc_for("nginx").display(),
        );
        std::fs::write(self.conf_path(paths), conf)?;
        // Provide a minimal fastcgi_params so the include resolves.
        std::fs::write(
            paths.etc_for("nginx").join("fastcgi_params"),
            "fastcgi_param QUERY_STRING $query_string;\n\
             fastcgi_param REQUEST_METHOD $request_method;\n\
             fastcgi_param CONTENT_TYPE $content_type;\n\
             fastcgi_param CONTENT_LENGTH $content_length;\n\
             fastcgi_param REQUEST_URI $request_uri;\n\
             fastcgi_param DOCUMENT_URI $document_uri;\n\
             fastcgi_param DOCUMENT_ROOT $document_root;\n\
             fastcgi_param SERVER_PROTOCOL $server_protocol;\n\
             fastcgi_param GATEWAY_INTERFACE CGI/1.1;\n\
             fastcgi_param REMOTE_ADDR $remote_addr;\n\
             fastcgi_param SERVER_NAME $server_name;\n",
        )?;
        Ok(())
    }
    fn command(&self, paths: &LaraluxPaths) -> SpawnSpec {
        SpawnSpec::new("nginx")
            .arg("-p")
            .arg(paths.etc_for("nginx").display().to_string())
            .arg("-c")
            .arg(self.conf_path(paths).display().to_string())
            .arg("-g")
            .arg("daemon off;")
    }
    fn health_check(&self, _paths: &LaraluxPaths) -> Result<(), ServiceError> {
        probe_tcp(self.http_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LaraluxPaths;
    use crate::service::{Service, ServiceKind};

    #[test]
    fn command_runs_nginx_with_prefix_and_daemon_off() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        let svc = NginxService::new("/tmp/lara/tmp/php-fpm.sock".into());
        let spec = svc.command(&p);
        assert_eq!(spec.program, "nginx");
        let joined = spec.args.join(" ");
        assert!(joined.contains("-p"));
        assert!(joined.contains("daemon off;"));
        assert_eq!(svc.kind(), ServiceKind::Nginx);
        assert_eq!(svc.deps(), &[ServiceKind::PhpFpm]);
    }

    #[test]
    fn write_config_wires_php_socket_and_includes_sites() {
        let tmp = std::env::temp_dir().join(format!("lara-nginx-{}", std::process::id()));
        let p = LaraluxPaths::new(tmp.clone());
        let sock = p.tmp().join("php-fpm.sock");
        let svc = NginxService::new(sock.clone());
        svc.write_config(&p).unwrap();
        let conf = std::fs::read_to_string(p.etc_for("nginx").join("nginx.conf")).unwrap();
        assert!(conf.contains(&format!("fastcgi_pass unix:{}", sock.display())));
        assert!(conf.contains("listen 80"));
        assert!(conf.contains("sites/*.conf"));
        // sites dir must exist so the glob include doesn't error.
        assert!(p.etc_for("nginx").join("sites").is_dir());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn write_config_includes_websocket_map() {
        let tmp = std::env::temp_dir().join(format!("lara-nginx-ws-{}", std::process::id()));
        let p = LaraluxPaths::new(tmp.clone());
        let svc = NginxService::new(p.tmp().join("php-fpm.sock"));
        svc.write_config(&p).unwrap();
        let conf = std::fs::read_to_string(p.etc_for("nginx").join("nginx.conf")).unwrap();
        assert!(conf.contains("map $http_upgrade $connection_upgrade"));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
