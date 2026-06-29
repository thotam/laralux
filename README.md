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

## License

[MIT](LICENSE) © thotam
