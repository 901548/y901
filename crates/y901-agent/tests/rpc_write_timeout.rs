//! `reply_to` 的写超时保护：客户端不读时，UI 线程不能被无限阻塞。
//!
//! 场景：RPC 客户端建立连接后不发也不读（输入法卡死、进程挂起、恶意连接）。
//! 若 socket 写超时未设置，`reply_to` 在 UI 线程内同步写会在缓冲写满后永久阻塞，
//! 整窗随即卡死。设 100ms 写超时 + 失败即 shutdown，把最坏阻塞压到 100ms。

use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use y901_agent::rpc::{reply_to, Pending, RpcRequest};
use y901_ui::Json;

/// 造一条"服务端已接受、客户端永不读取"的连接。
fn stalled_pair() -> (Arc<Mutex<TcpStream>>, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let client = TcpStream::connect(addr).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    // 与生产代码一致的写超时
    server
        .set_write_timeout(Some(Duration::from_millis(100)))
        .expect("set_write_timeout");
    // 客户端把接收缓冲设小，尽快触发写阻塞
    let _ = client.set_read_timeout(Some(Duration::from_millis(50)));
    (Arc::new(Mutex::new(server)), client)
}

fn pending_on(writer: Arc<Mutex<TcpStream>>) -> Pending {
    let (tx, _rx) = channel();
    Pending {
        request: RpcRequest::Ping,
        id: Json::Num(1.0),
        reply: tx,
        writer,
    }
}

#[test]
fn reply_to_does_not_block_when_client_never_reads() {
    let (server, _client) = stalled_pair();
    // 单次回包远小于缓冲，先确认正常情况下秒回
    let t0 = Instant::now();
    reply_to(pending_on(server.clone()), Ok(Json::obj().set("probe", true)));
    assert!(
        t0.elapsed() < Duration::from_millis(50),
        "小回包不应阻塞，实际 {:?}",
        t0.elapsed()
    );

    // 灌到缓冲写满：4MiB 必然超过内核发送缓冲 + 对端接收窗
    let big = "x".repeat(4 * 1024 * 1024);
    let t1 = Instant::now();
    reply_to(pending_on(server.clone()), Ok(Json::obj().set("blob", big)));
    let blocked = t1.elapsed();

    assert!(
        blocked < Duration::from_millis(400),
        "写超时必须生效：最坏阻塞应 <400ms，实际 {blocked:?}"
    );
}

#[test]
fn failed_write_shuts_the_connection_down() {
    let (server, mut client) = stalled_pair();
    let big = "x".repeat(4 * 1024 * 1024);
    reply_to(pending_on(server.clone()), Ok(Json::obj().set("blob", big)));

    // shutdown(Both) 后，客户端读到的应是 EOF（0 字节），而不是继续等待
    let mut buf = [0u8; 64];
    let mut eof = false;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        match client.read(&mut buf) {
            Ok(0) => {
                eof = true;
                break;
            }
            Ok(_) => continue, // 先把已进入缓冲的字节读掉
            Err(_) => {
                eof = true; // 连接被重置也算断开
                break;
            }
        }
    }
    assert!(eof, "写失败后连接应被 shutdown（客户端应读到 EOF）");

    // 断开后再次回包应立即失败（不会再等满 100ms）
    let t = Instant::now();
    reply_to(pending_on(server), Ok(Json::obj().set("after", "shutdown")));
    assert!(
        t.elapsed() < Duration::from_millis(100),
        "已关闭连接上的写应立即失败，实际 {:?}",
        t.elapsed()
    );
    // 注意：不再断言"客户端写入失败" —— 那取决于内核何时回 RST，
    // 是操作系统行为而非本模块契约，断言它只会制造平台相关的假红。
}
