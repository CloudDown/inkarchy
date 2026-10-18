use super::*;

#[test]
fn switching_notebooks_cancels_a_pending_network_future_promptly() {
    let transport = Transport::new().unwrap();
    let epoch = std::sync::Arc::new(AtomicU64::new(1));
    let change = epoch.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        change.store(2, Ordering::Relaxed);
    });
    let start = std::time::Instant::now();
    let result: Result<(), String> = transport.run(&epoch, 1, std::future::pending());
    assert!(result.unwrap_err().contains("cancelled"));
    assert!(start.elapsed() < Duration::from_millis(500));
    thread.join().unwrap();
}

fn result() -> Value {
    json!({"exercise_found":true,"readable":true,"complete_step":true,"verdict":"on_track",
        "statement":"Solve 2x+3=11", "recognized_step":"2x=8", "explanation":"Valid subtraction.","region":null})
}

#[test]
fn both_provider_contracts_decode_and_reject_truncated_or_invalid_outputs() {
    let content = result().to_string();
    let ollama = json!({"done":true,"done_reason":"stop","message":{"content":content}});
    assert!(decode(Provider::Ollama, ollama.clone()).is_ok());
    let mut short = ollama;
    short["done_reason"] = json!("length");
    assert!(decode(Provider::Ollama, short).is_err());
    let gemini = json!({"candidates":[{"finishReason":"STOP","content":{"parts":[{"thought":true,"text":"ignore"},{"text":content}]}}]});
    assert!(decode(Provider::Gemini, gemini.clone()).is_ok());
    let mut short = gemini;
    short["candidates"][0]["finishReason"] = json!("MAX_TOKENS");
    assert!(decode(Provider::Gemini, short).is_err());
    assert!(decode(
        Provider::Ollama,
        json!({"done":true,"message":{"content":"not JSON"}})
    )
    .is_err());
    assert!(decode(Provider::Gemini, json!({"candidates":[]})).is_err());
}

#[test]
fn requests_contain_images_schema_and_bounded_generation_without_credentials() {
    for provider in [Provider::Gemini, Provider::Ollama] {
        let settings = AiSettings {
            provider,
            ..Default::default()
        };
        let payload = body(&settings, &[vec![1, 2, 3]], None, "Image 1: full page.");
        let serialized = payload.to_string();
        assert!(serialized.contains("AQID"));
        assert!(serialized.contains("complete_step"));
        assert!(!serialized.contains("api_key"));
        assert!(!serialized.contains("enabled_notebooks"));
    }
}

#[test]
fn local_mode_rejects_remote_hosts_cloud_models_and_credential_urls() {
    for url in [
        "https://ollama.com",
        "http://127.0.0.1.evil.test",
        "http://key@localhost:11434",
        "file:///tmp/ai",
    ] {
        let settings = AiSettings {
            local_url: url.into(),
            ..Default::default()
        };
        assert!(local_base(&settings).is_err());
    }
    for url in [
        "http://localhost:11434",
        "http://127.0.0.1:11434",
        "http://[::1]:11434",
    ] {
        assert!(local_base(&AiSettings {
            local_url: url.into(),
            ..Default::default()
        })
        .is_ok());
    }
    assert!(local_base(&AiSettings {
        local_model: "qwen3-vl:235b-cloud".into(),
        ..Default::default()
    })
    .is_err());
}

#[test]
fn http_errors_are_safe_and_timeout_is_reported() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    for (status, expected) in [
        (401, "key"),
        (403, "key"),
        (404, "Model"),
        (429, "quota"),
        (503, "unavailable"),
        (200, "invalid JSON"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf);
            write!(stream,"HTTP/1.1 {status} Test\r\nContent-Length: 17\r\nConnection: close\r\n\r\nprivate-key-value").unwrap();
        });
        let response = client().unwrap().get(url).send().unwrap();
        let error = response_json(response).unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("private-key"));
        server.join().unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(80));
    });
    let error = client()
        .unwrap()
        .get(url)
        .timeout(Duration::from_millis(20))
        .send()
        .unwrap_err();
    assert!(network_error(error).contains("timed out"));
    server.join().unwrap();
}
