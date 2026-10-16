//! 测试用原始 HTTP mock 基建（加固版）。
//!
//! 背景：既有的各测试模块手写 mock 服务器「单次 `read` 即应答」——只消费第一个
//! TCP 段。当请求头与请求体分段到达（宿主重启后网络栈/安全软件干预下必现）时，
//! 服务器在客户端尚未写完请求体前就应答并关闭，客户端报「error reading a body
//! from connection / end of file before message length reached」等随机失败。
//!
//! 修复：完整读完请求（头 + Content-Length 指定的体）后再应答。所有 mock 站点
//! 统一调用本模块，消除同一隐雷的 12 处复制。

use tokio::io::AsyncReadExt;

/// 完整读取一条 HTTP/1.x 请求（头 + Content-Length 指定的体），返回原始字节。
/// 读不到任何字节（对端提前关闭）或出错时返回已读部分，调用方按既有
/// best-effort 语义处理（不 panic、不重试）。
pub(crate) async fn read_request_full(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    // 1) 读到 "\r\n\r\n" 头结束
    let header_end = loop {
        if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        match socket.read(&mut tmp).await {
            Ok(0) | Err(_) => return buf,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    };
    // 2) 解析 Content-Length，继续读体（无该头时视为 0）
    let content_len = content_length(&buf[..header_end]).unwrap_or(0);
    while buf.len() < header_end + content_len {
        match socket.read(&mut tmp).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    buf
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=(haystack.len() - needle.len())).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// keep-alive 安全的请求读取：从 `buf`（跨请求持久）中解析出「一条完整 HTTP/1.x 请求」，
/// 返回 `(该请求字节, 剩余未消费字节)`。剩余字节会被下一次调用继续消费，避免多请求共用
/// 一个连接时「多读出的下一条请求字节被丢弃 → 连接错位卡死」。
///
/// `buf` 必须由调用方在多次调用间保持并持续追加（`socket.read` 的数据）。本函数只在
/// 确实凑齐一条请求时才返回 `Some`，否则继续从 socket 读，直到对端关闭（`None`）。
pub(crate) async fn read_request_keepalive(
    socket: &mut tokio::net::TcpStream,
    buf: &mut Vec<u8>,
) -> Option<Vec<u8>> {
    let mut tmp = [0u8; 4096];
    loop {
        // 头结束
        let header_end = match find_subsequence(buf, b"\r\n\r\n") {
            Some(pos) => pos + 4,
            None => {
                match socket.read(&mut tmp).await {
                    Ok(0) | Err(_) => return None,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
                continue;
            }
        };
        // 体长度
        let content_len = content_length(&buf[..header_end]).unwrap_or(0);
        if buf.len() < header_end + content_len {
            match socket.read(&mut tmp).await {
                Ok(0) | Err(_) => return None,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
            }
            continue;
        }
        // 恰好一条完整请求：切出，剩余留在 buf 供下一请求
        let req_end = header_end + content_len;
        let req: Vec<u8> = buf[..req_end].to_vec();
        let rest: Vec<u8> = buf[req_end..].to_vec();
        buf.clear();
        buf.extend_from_slice(&rest);
        return Some(req);
    }
}

/// 大小写不敏感地解析 Content-Length 头（无则 None）。
fn content_length(headers: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(headers).to_ascii_lowercase();
    let key = "content-length:";
    let pos = text.find(key)?;
    let rest = &text[pos + key.len()..];
    rest.trim_start()
        .split(['\r', '\n', ' ', '\t'])
        .next()?
        .parse::<usize>()
        .ok()
}
