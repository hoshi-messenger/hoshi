use std::net::SocketAddr;

use tokio::net::{TcpListener, TcpSocket};

pub fn create_listener(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    socket.set_reuseaddr(true)?;
    socket.bind(addr)?;
    socket.listen(1024)
}

pub fn create_http_listener(
    addr: SocketAddr,
) -> std::io::Result<(TcpListener, SocketAddr)> {
    let http_listener = create_listener(addr)?;
    let http_addr = http_listener.local_addr()?;
    Ok((http_listener, http_addr))
}

pub fn systemd_notify_ready_with_watchdog() {
    #[cfg(target_os = "linux")]
    {
        use sd_notify::{NotifyState, notify};
        use std::time::Duration;
        use tokio::time::interval;

        let _ = notify(false, &[NotifyState::Ready]);

        let watchdog_usec = match std::env::var("WATCHDOG_USEC") {
            Ok(v) => v.parse::<u64>().ok(),
            Err(_) => None,
        };

        let watchdog_usec = match watchdog_usec {
            Some(v) if v > 0 => v,
            _ => return,
        };

        let interval_duration = Duration::from_micros(watchdog_usec / 2);

        tokio::spawn(async move {
            let mut ticker = interval(interval_duration);
            loop {
                ticker.tick().await;
                let _ = notify(false, &[NotifyState::Watchdog]);
            }
        });
    }
}
