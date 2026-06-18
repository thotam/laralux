use laragon_core::{build_services, Config, LaragonPaths, Orchestrator, RealSpawner};

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    let paths = LaragonPaths::new(LaragonPaths::default_root());

    match cmd.as_str() {
        "config-init" => {
            paths.ensure_dirs().expect("create dirs");
            let cfg = Config::default();
            cfg.save(&paths.config_file()).expect("save config");
            println!("Initialized {}", paths.config_file().display());
        }
        "up" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            paths.ensure_dirs().expect("create dirs");
            let mut orch =
                Orchestrator::new(paths.clone(), build_services(&cfg, &paths), Box::new(RealSpawner));
            match orch.start_all() {
                Ok(()) => println!("Started all services. Press Ctrl-C to stop."),
                Err(e) => {
                    eprintln!("start failed: {e}");
                    orch.stop_all();
                    std::process::exit(1);
                }
            }
            // Keep the process (and thus child processes) alive until Ctrl-C.
            wait_for_ctrl_c();
            println!("Stopping...");
            orch.stop_all();
        }
        "status" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            let orch =
                Orchestrator::new(paths.clone(), build_services(&cfg, &paths), Box::new(RealSpawner));
            for kind in orch.start_order() {
                println!("{:?}: {:?}", kind, orch.state(kind));
            }
        }
        "down" => {
            println!("`up` manages the process lifetime; stop it with Ctrl-C.");
        }
        _ => {
            println!("usage: laragonctl <config-init|up|status>");
        }
    }
}

fn wait_for_ctrl_c() {
    use std::sync::mpsc::channel;
    let (tx, rx) = channel();
    ctrlc_lite(move || {
        let _ = tx.send(());
    });
    let _ = rx.recv();
}

// Minimal Ctrl-C handler without external crates: register a SIGINT handler
// that writes to a static flag via a self-pipe-free approach using libc.
fn ctrlc_lite<F: Fn() + Send + 'static>(f: F) {
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static HANDLER: OnceLock<Mutex<Option<Box<dyn Fn() + Send>>>> = OnceLock::new();
    HANDLER.get_or_init(|| Mutex::new(None));
    *HANDLER.get().unwrap().lock().unwrap() = Some(Box::new(f));

    extern "C" fn on_sigint(_sig: i32) {
        if let Some(lock) = HANDLER.get() {
            if let Ok(guard) = lock.lock() {
                if let Some(cb) = guard.as_ref() {
                    cb();
                }
            }
        }
    }
    extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }
    unsafe {
        signal(2, on_sigint); // SIGINT = 2
    }
}
