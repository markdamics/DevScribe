use std::io::{BufRead, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

pub fn run(socket_path: &Path) {
    let mut request = String::new();
    if std::io::stdin().read_to_string(&mut request).is_err() {
        deny("devscribe: couldn't read the tool call from stdin");
        return;
    }

    match forward(socket_path, request.trim()) {
        Ok(decision) => println!("{decision}"),
        Err(reason) => deny(&reason),
    }
}

fn forward(socket_path: &Path, request: &str) -> Result<String, String> {
    let mut stream =
        UnixStream::connect(socket_path).map_err(|err| format!("devscribe: couldn't reach the running session: {err}"))?;
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.write_all(b"\n"))
        .map_err(|err| format!("devscribe: couldn't send the request: {err}"))?;

    let mut decision = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut decision)
        .map_err(|err| format!("devscribe: couldn't read the decision: {err}"))?;
    if decision.trim().is_empty() {
        return Err("devscribe: no decision received".to_string());
    }
    Ok(decision.trim().to_string())
}

fn deny(reason: &str) {
    let response = serde_json::json!({"decision": "block", "reason": reason});
    println!("{response}");
}
