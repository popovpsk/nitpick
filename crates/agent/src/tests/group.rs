use std::time::{Duration, Instant};

pub(super) async fn ps_state(pid: &str) -> Option<String> {
    let output = tokio::process::Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .await
        .unwrap();
    let state = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!state.is_empty()).then_some(state)
}

pub(super) async fn assert_dies(pid: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match ps_state(pid).await {
            None => return,
            Some(state) if state.starts_with('Z') => return,
            Some(_) if Instant::now() > deadline => panic!("process {pid} is still alive"),
            Some(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}
