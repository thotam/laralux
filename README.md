# Laralux

A native Linux local web-development environment manager: a system-tray + GUI
manager for a local web-development stack — **nginx, PHP-FPM, MariaDB,
PostgreSQL, MongoDB, Redis, Mailpit** — with pretty `*.dev` HTTPS, automatic
[mkcert](https://github.com/FiloSottile/mkcert) SSL, multi-version tool
switching, and per-site Procfile process management.

Tool binaries are downloaded as **static builds into `~/laralux/`** at your
request at runtime — no `apt` needed for the managed stack.

## Install (Debian/Ubuntu, x86_64)

Download the latest `laralux_<version>_amd64.deb` from the
[Releases page](https://github.com/thotam/laralux/releases) and install it with
apt (it resolves the runtime dependencies automatically):

```sh
sudo apt install ./laralux_<version>_amd64.deb
```

To remove:

```sh
sudo apt remove laralux
```

## Public domains

Each site can also serve one or more real domains, for when an upstream
server terminates public TLS (e.g. Let's Encrypt) and reverse-proxies down to
the device running laralux. Add them via the site's row menu → **Public
domains**.

laralux serves each public domain on **both port 80 and 443**, with no
HTTP→HTTPS redirect, and does not add them to `/etc/hosts`. On port 443 the
device uses the site's [mkcert](https://github.com/FiloSottile/mkcert)
certificate — locally-trusted only — which is extended to cover the public
domains.

On the **upstream** server (example nginx), terminate the real public TLS and
proxy over HTTPS to the device. Since the device's certificate is mkcert (not
publicly trusted), set `proxy_ssl_verify off`:

```nginx
server {
  listen 443 ssl;
  server_name app.example.com;
  # ssl_certificate ... ;      # Let's Encrypt on the upstream
  # ssl_certificate_key ... ;

  location / {
    proxy_pass https://<device-ip>:443;   # e.g. https://192.168.0.18:443
    proxy_ssl_verify off;                  # device serves a local mkcert cert
    proxy_set_header Host              $host;
    proxy_set_header X-Real-IP         $remote_addr;
    proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto https;
  }
}
```

Proxying to `http://<device-ip>:80` instead also works, since the device
serves the public domain on both ports.

In the Laravel app, configure
[`TrustProxies`](https://laravel.com/docs/requests#configuring-trusted-proxies)
so the `X-Forwarded-*` headers from the upstream are honoured.

## License

[MIT](LICENSE) © thotam
