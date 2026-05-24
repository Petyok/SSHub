use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct PingResult {
    pub host_name: String,
    pub address: String,
    pub latency_ms: Option<u32>, // None = timeout/unreachable
}

/// Spawn a background thread that pings the given addresses every `interval`.
/// Returns a Receiver that yields PingResult as they come in.
pub fn spawn_ping_worker(hosts: Vec<(String, String)>, interval: Duration) -> Receiver<PingResult> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        ping_loop(hosts, interval, tx);
    });
    rx
}

fn ping_loop(hosts: Vec<(String, String)>, interval: Duration, tx: Sender<PingResult>) {
    loop {
        for (name, address) in &hosts {
            let result = ping_once(name, address);
            if tx.send(result).is_err() {
                return; // Receiver dropped, exit thread
            }
        }
        thread::sleep(interval);
    }
}

fn ping_once(name: &str, address: &str) -> PingResult {
    // Use `ping -c 1 -W 1` (1 attempt, 1 second timeout)
    let output = Command::new("ping")
        .args(["-c", "1", "-W", "1", address])
        .output();

    let latency_ms = match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            parse_ping_time(&stdout)
        }
        _ => None,
    };

    PingResult {
        host_name: name.to_string(),
        address: address.to_string(),
        latency_ms,
    }
}

fn parse_ping_time(output: &str) -> Option<u32> {
    // Linux: "time=12.3 ms"  macOS: "time=12.345 ms"
    for line in output.lines() {
        if let Some(pos) = line.find("time=") {
            let after = &line[pos + 5..];
            let num_str: String = after
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(ms) = num_str.parse::<f64>() {
                return Some(ms.round() as u32);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_linux_ping() {
        let out = "64 bytes from 10.0.0.1: icmp_seq=1 ttl=64 time=12.3 ms";
        assert_eq!(parse_ping_time(out), Some(12));
    }

    #[test]
    fn parse_macos_ping() {
        let out = "64 bytes from 10.0.0.1: icmp_seq=0 ttl=64 time=0.847 ms";
        assert_eq!(parse_ping_time(out), Some(1));
    }

    #[test]
    fn parse_no_match() {
        assert_eq!(parse_ping_time("Request timeout"), None);
    }

    #[test]
    fn parse_multiline_output() {
        let out = "PING 10.0.0.1 (10.0.0.1) 56(84) bytes of data.\n\
                   64 bytes from 10.0.0.1: icmp_seq=1 ttl=64 time=3.45 ms\n\
                   \n\
                   --- 10.0.0.1 ping statistics ---\n\
                   1 packets transmitted, 1 received, 0% packet loss, time 0ms\n\
                   rtt min/avg/max/mdev = 3.450/3.450/3.450/0.000 ms";
        assert_eq!(parse_ping_time(out), Some(3));
    }
}
