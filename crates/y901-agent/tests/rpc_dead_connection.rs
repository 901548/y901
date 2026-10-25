//! B+（零阻塞 + 失败即断开）的四个边界。
//!
//! 核心契约：
//! 1. UI 线程**永不阻塞**：`reply_to` 只看 `dead` + 一次 `try_send`，不轮询、不睡眠。
//! 2. 队列溢出的连接被标记 `dead` 并 shutdown，客户端立刻收到 EOF。
//! 3. 写线程有写超时，才能周期性醒来检查 `dead`（否则永久阻塞在 writeln）。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use y901_agent::rpc::{reply_to, spawn, Pending, RpcRequest};
use y901_ui::Json;

fn send_ping(w: &mut TcpStream, id: u32) -> std::io::Result<()> {
    writeln!(w, r#"{{"id":{id},"method":"ping"}}"#)?;
    w.flush()
}

/// 测试 1：客户端连上不读，灌满写队列 → 连接被断开，客户端读到 EOF。
#[test]
fn stalled_client_gets_disconnected() {
    let (tx, rx) = std::sync::mpsc::channel();
    let (addr, hub) = spawn("127.0.0.1:0", tx).expect("bind");

    let client = TcpStream::connect(addr).expect("connect");
    let mut reader = BufReader::new(client.try_clone().expect("clone"));
    let mut writer = client;

    // 持续发请求直到写失败（连接被服务端断开）：不依赖"2000 条能在 X ms 内发完"
    std::thread::spawn(move || {
        for id in 0..100_000u32 {
            if send_ping(&mut writer, id).is_err() {
                break; // EOF / EPIPE：服务端已断开
            }
        }
        let _ = writer.shutdown(std::net::Shutdown::Both);
    });

    // UI 线程：只要能拿到请求就回包（队列满了 reply_to 会返回 Err 并判死）
    let ui = std::thread::spawn(move || {
        let mut handled = 0;
        while let Ok(pending) = rx.recv_timeout(Duration::from_millis(500)) {
            let _ = reply_to(pending, Ok(Json::obj().set("tag", "pong")));
            handled += 1;
            if handled >= 100_000 {
                break;
            }
        }
        handled
    });

    // 客户端最终应看到 EOF（连接被服务端断开）
    let mut buf = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut eof = false;
    while Instant::now() < deadline {
        match reader.read(&mut buf) {
            Ok(0) => {
                eof = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => {
                eof = true;
                break;
            }
        }
    }
    assert!(eof, "队列溢出后连接必须被断开（客户端应读到 EOF）");

    ui.join().unwrap();

    // 连接应被注销
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && hub.connection_count() > 0 {
        hub.send("{\"event\":\"probe\"}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(hub.connection_count(), 0, "死连接必须被注销");
}

/// 测试 2：`dead` 已置位的连接，`reply_to` 立即返回 Err（UI 零阻塞）。
#[test]
fn dead_flag_prevents_further_try_send() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let (_addr, _hub) = spawn("127.0.0.1:0", tx).expect("bind");

    // 构造一个已判死的 Pending：队列容量 1 且塞满
    let (job_tx, _job_rx) = std::sync::mpsc::sync_channel(1);
    job_tx
        .try_send(y901_agent::rpc::WriteJob::Push("x".into()))
        .expect("fill");
    let dead = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));

    let pending = Pending {
        request: RpcRequest::Ping,
        id: Json::Num(1.0),
        writer_tx: job_tx,
        dead,
    };

    let t = Instant::now();
    let r = reply_to(pending, Ok(Json::obj().set("tag", "pong")));
    let elapsed = t.elapsed();

    assert!(r.is_err(), "dead 连接的回包必须返回 Err");
    assert!(
        elapsed < Duration::from_millis(1),
        "dead 检查必须在 1ms 内返回（不做 try_send），实际 {elapsed:?}"
    );
}

/// 测试 3：读得慢但仍在读的客户端，能拿到部分回包（不是一上来就断）。
#[test]
fn slow_client_still_gets_replies_until_dead() {
    let (tx, rx) = std::sync::mpsc::channel();
    let (addr, _hub) = spawn("127.0.0.1:0", tx).expect("bind");

    let client = TcpStream::connect(addr).expect("connect");
    client.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut reader = BufReader::new(client.try_clone().expect("clone"));
    let mut writer = client;

    let n = 20u32;
    for id in 0..n {
        let _ = send_ping(&mut writer, id);
    }

    let ui = std::thread::spawn(move || {
        let mut sent = 0;
        while let Ok(pending) = rx.recv_timeout(Duration::from_millis(500)) {
            if reply_to(pending, Ok(Json::obj().set("tag", "pong"))).is_ok() {
                sent += 1;
            }
            if sent >= 20 {
                break;
            }
        }
        sent
    });

    // 慢读：每行之间睡 10ms
    let mut got = 0;
    for _ in 0..n {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if Json::parse(&line).is_ok() {
            got += 1;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    ui.join().unwrap();
    assert!(got > 0, "慢读客户端至少应收到部分回包");
}

/// 测试 4：写线程必须设写超时（否则检查不到 dead）。
///
/// 行为断言：`configure_stream` 之后 `write_timeout()` 必须读回 `Some(100ms)`。
/// 没设 → 返回 `None`，测试立刻红。
#[test]
fn write_timeout_is_set_on_connection() {
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let _client = TcpStream::connect(addr).expect("connect");
    let (server, _) = listener.accept().expect("accept");

    // 未配置前：系统默认无写超时
    assert_eq!(
        server.write_timeout().expect("query"),
        None,
        "前置：新连接不应自带写超时"
    );

    y901_agent::rpc::configure_stream(&server).expect("configure");
    assert_eq!(
        server.write_timeout().expect("query"),
        Some(std::time::Duration::from_millis(100)),
        "写线程必须设置写超时（100ms），否则会永久阻塞在 writeln"
    );
}
