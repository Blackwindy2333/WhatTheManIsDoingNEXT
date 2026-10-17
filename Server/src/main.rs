//! 服务器入口：装载配置 → 建状态 → 起 HTTP 服务（优雅退出）。

use std::net::SocketAddr;

use wtmd_server::config::Config;
use wtmd_server::routes;

fn main() {
    let (config, config_path) = Config::load_workspace_config();
    eprintln!(
        "[wtmd-server] config loaded from {}",
        config_path.display()
    );

    let state = match wtmd_server::AppState::new(config.clone()) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("[wtmd-server] failed to open database {}: {err}", config.db_path);
            std::process::exit(1);
        }
    };

    let app = routes::router(state);
    let web_dir = routes::resolve_web_dir(&config.web_dir);
    let addr = format!("{}:{}", config.host, config.port);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("[wtmd-server] failed to start tokio runtime: {err}");
            std::process::exit(1);
        }
    };

    runtime.block_on(async move {
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("[wtmd-server] failed to bind {addr}: {err}");
                std::process::exit(1);
            }
        };
        eprintln!("[wtmd-server] listening on http://{addr}");
        eprintln!("[wtmd-server] serving web UI from {}", web_dir.display());
        if let Err(err) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await
        {
            eprintln!("[wtmd-server] server error: {err}");
            std::process::exit(1);
        }
        eprintln!("[wtmd-server] shutdown complete");
    });
}

async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_ok() {
        eprintln!("[wtmd-server] ctrl-c received, shutting down");
    }
}
