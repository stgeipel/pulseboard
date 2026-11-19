use pulseboard::{
    config, monitor,
    monitor::{Command, Event, Outcome},
};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    time::Duration,
};

#[tokio::test]
async fn sends_post_with_payload_and_credentials_and_keeps_valid_checks_after_bad_reload() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("config.toml");
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    fs::write(
        &file,
        format!(
            r#"
            interval_seconds = 3600

            [[checks]]
            id = "test"
            name = "Test"
            url = "http://{address}/health"
            method = "POST"
            expected_status = [302]
            body = 'hello'
            [checks.headers]
            Content-Type = "text/plain"
            [checks.basic_auth]
            username = "monitor"
            password = "test-password"
            "#
        ),
    )
    .unwrap();
    let remote = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let redirect_address = remote.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut request, headers) = read_request(&listener);
            let lower_headers = headers.to_ascii_lowercase();
            assert!(headers.starts_with("POST /health HTTP/1.1"));
            assert!(lower_headers.contains("content-type: text/plain"));
            assert!(lower_headers.contains("authorization: basic "));
            assert!(headers.contains("bW9uaXRvcjp0ZXN0LXBhc3N3b3Jk"));
            let mut body = [0; 5];
            request.read_exact(&mut body).unwrap();
            assert_eq!(&body, b"hello");
            write!(
                request.get_mut(),
                "HTTP/1.1 302 Found\r\nLocation: http://{redirect_address}/other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        }
    });

    let (commands, events) = monitor::start(file.clone()).unwrap();
    let next = || async {
        tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap()
    };
    assert!(matches!(next().await, Event::Configured(config) if config.checks.len() == 1));
    assert!(matches!(next().await, Event::Started));
    assert!(
        matches!(next().await, Event::Finished(results, _) if matches!(results[0].outcome, Outcome::Healthy(302)))
    );

    fs::write(&file, "interval_seconds = -1").unwrap();
    commands.send(Command::Reload).await.unwrap();
    assert!(matches!(next().await, Event::ConfigError(_)));
    commands.send(Command::Run).await.unwrap();
    assert!(matches!(next().await, Event::Started));
    assert!(
        matches!(next().await, Event::Finished(results, _) if matches!(results[0].outcome, Outcome::Healthy(302)))
    );
    server.join().unwrap();
    remote.set_nonblocking(true).unwrap();
    assert!(
        matches!(remote.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[tokio::test]
async fn probes_get_on_schedule_and_detects_recovery_without_a_manual_command() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("config.toml");
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    fs::write(
        &file,
        format!(
            r#"
            interval_seconds = 1

            [[checks]]
            id = "test"
            name = "Test"
            url = "http://{address}/health"
            "#
        ),
    )
    .unwrap();
    let server = std::thread::spawn(move || {
        for status in ["503 Service Unavailable", "204 No Content"] {
            let (mut request, headers) = read_request(&listener);
            assert!(headers.starts_with("GET /health HTTP/1.1"));
            write!(
                request.get_mut(),
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        }
    });
    let (_commands, events) = monitor::start(file).unwrap();
    let outcomes = tokio::time::timeout(Duration::from_secs(5), async {
        let mut outcomes = Vec::new();
        while outcomes.len() < 2 {
            if let Event::Finished(mut results, _) = events.recv().await.unwrap() {
                outcomes.push(results.pop().unwrap().outcome);
            }
        }
        outcomes
    })
    .await
    .unwrap();
    assert!(matches!(
        outcomes.as_slice(),
        [Outcome::Unhealthy(503), Outcome::Healthy(204)]
    ));
    server.join().unwrap();
}

fn read_request(listener: &TcpListener) -> (BufReader<TcpStream>, String) {
    let (socket, _) = listener.accept().unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut request = BufReader::new(socket);
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        assert!(request.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        headers.push_str(&line);
    }
    (request, headers)
}

#[test]
fn configuration_errors_omit_credentials() {
    let config = r#"
    [[checks]]
    id = "test"
    name = "Test"
    url = "https://example.com"
    [checks.headers]
    Authorization = "test-secret" @
    "#;
    let result = config::parse(config);
    let Err(error) = result else {
        panic!("invalid TOML accepted");
    };
    assert!(!format!("{error:#}").contains("test-secret"));
}
