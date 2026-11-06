//! Minimal HTTP/1.1 server replaying canned Drive responses in order.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub(crate) struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(status: u16, body: serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![("Content-Type", "application/json".into())],
            body: body.to_string().into_bytes(),
        }
    }

    pub fn bytes(body: &[u8]) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type", "application/octet-stream".into())],
            body: body.to_vec(),
        }
    }
}

pub(crate) struct FakeDrive {
    pub base: String,
    /// "METHOD /path?query" of every request, in arrival order.
    pub requests: Arc<Mutex<Vec<String>>>,
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0)
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut remaining = content_length(&head).saturating_sub(buf.len() - head_end);
    while remaining > 0 {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        remaining = remaining.saturating_sub(n);
    }
    head.lines()
        .next()
        .map(|l| l.rsplitn(2, ' ').last().unwrap_or(l).to_string())
}

impl FakeDrive {
    pub async fn start(replies: Vec<Reply>) -> Self {
        Self::start_with(|_| replies).await
    }

    /// Replies may reference the server address, e.g. in a `Location` header.
    pub async fn start_with(replies: impl FnOnce(&str) -> Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let replies = replies(&base);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();

        tokio::spawn(async move {
            let mut replies = replies.into_iter();
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                while let Some(line) = read_request(&mut stream).await {
                    log.lock().unwrap().push(line);
                    let Some(reply) = replies.next() else { return };
                    let mut head = format!(
                        "HTTP/1.1 {} X\r\nContent-Length: {}\r\n",
                        reply.status,
                        reply.body.len()
                    );
                    for (k, v) in &reply.headers {
                        head.push_str(&format!("{k}: {v}\r\n"));
                    }
                    head.push_str("\r\n");
                    if stream.write_all(head.as_bytes()).await.is_err()
                        || stream.write_all(&reply.body).await.is_err()
                    {
                        break;
                    }
                }
            }
        });

        Self { base, requests }
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}
