/// Tests API behavior under stress/edge conditions.
/// Requires: running daemon with valid API key.
#[cfg(test)]
mod api_stress {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    /// Verify the daemon responds to a health/RPC ping.
    #[test]
    #[cfg_attr(not(feature = "integration-tests"), ignore)]
    fn daemon_responds_to_ping() {
        let mut stream =
            UnixStream::connect("/run/aletheon/aletheon.sock").expect("Should connect");

        // Send a basic JSON-RPC ping/initialize
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": {"client": "integration-test"},
            "id": 1
        });
        let mut req_str = serde_json::to_string(&request).unwrap();
        req_str.push('\n');
        stream.write_all(req_str.as_bytes()).expect("Should write");

        let mut buf = [0u8; 4096];
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("Should set timeout");
        let n = stream.read(&mut buf).expect("Should read response");
        assert!(n > 0, "Should get a response from daemon");

        let response: serde_json::Value =
            serde_json::from_slice(&buf[..n]).expect("Response should be valid JSON");
        assert!(
            response.get("result").is_some() || response.get("error").is_some(),
            "Response should have result or error field"
        );
    }

    /// Verify socket handles rapid connect/disconnect gracefully.
    #[test]
    #[cfg_attr(not(feature = "integration-tests"), ignore)]
    fn rapid_connect_disconnect() {
        for _ in 0..10 {
            let _stream =
                UnixStream::connect("/run/aletheon/aletheon.sock").expect("Should connect");
            // Drop immediately — daemon shouldn't crash or leak
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
