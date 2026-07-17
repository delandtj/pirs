use futures_util::StreamExt;

pub struct SseStream {
    inner: futures_util::stream::BoxStream<'static, Result<String, crate::AiError>>,
}

impl SseStream {
    pub fn new(response: reqwest::Response) -> Self {
        let stream = response
            .bytes_stream()
            .scan(String::new(), |buf, chunk| {
                let mut events = Vec::new();
                match chunk {
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some((pos, delim_len)) = find_event_boundary(buf) {
                            let raw: String = buf.drain(..pos + delim_len).collect();
                            let body = raw[..pos].trim();
                            if body.is_empty() {
                                continue;
                            }
                            let mut data = String::new();
                            for line in body.lines() {
                                if let Some(rest) = line.strip_prefix("data:") {
                                    if !data.is_empty() {
                                        data.push('\n');
                                    }
                                    data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                                }
                            }
                            if !data.is_empty() {
                                events.push(Ok(data));
                            }
                        }
                    }
                    Err(e) => events.push(Err(crate::AiError::Network(e))),
                }
                std::future::ready(Some(futures_util::stream::iter(events)))
            })
            .flatten()
            .boxed();
        SseStream { inner: stream }
    }

    pub async fn next(&mut self) -> Option<Result<String, crate::AiError>> {
        self.inner.next().await
    }
}

fn find_event_boundary(buf: &str) -> Option<(usize, usize)> {
    let crlf = buf.find("\r\n\r\n").map(|p| (p, 4));
    let lf = buf.find("\n\n").map(|p| (p, 2));
    match (crlf, lf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_lf() {
        assert_eq!(find_event_boundary("a\n\nb"), Some((1, 2)));
    }

    #[test]
    fn boundary_crlf() {
        assert_eq!(find_event_boundary("a\r\n\r\nb"), Some((1, 4)));
    }

    #[test]
    fn none() {
        assert_eq!(find_event_boundary("a\nb"), None);
    }

    #[test]
    fn parse_single_event() {
        let mut buf = String::from("data: hello\n\nrest");
        let (pos, len) = find_event_boundary(&buf).unwrap();
        let raw: String = buf.drain(..pos + len).collect();
        assert_eq!(raw, "data: hello\n\n");
        assert_eq!(buf, "rest");
    }
}
