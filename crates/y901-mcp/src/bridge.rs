//! stdio ↔ UI 主线程的双向桥。
//!
//! 三个角色，互不阻塞：
//! ```text
//! stdin 线程 ──job──▶ UI 线程（唯一持有 UiTree）──reply──▶ stdout 线程
//! ```
//! - UI 线程只做 `try_recv`，永远不碰 stdin；
//! - stdin 线程在"等 UI 回复"时阻塞，这是它该干的事；
//! - stdout 线程串行写、逐行 flush（MCP 客户端按行读）。

use std::io::{BufRead, Write};
use std::sync::mpsc::{channel, Receiver, Sender};

use y901_agent::McpServer;
use y901_ui::UiTree;

/// 一次请求：原始行 + 回传通道。
pub struct Job {
    pub line: String,
    pub reply: Sender<Option<String>>,
}

/// 启动 IO 线程，返回 UI 线程要消费的 job 队列。
///
/// `input` 通常是 `stdin`，`output` 是 `stdout`；测试里可以换成内存缓冲。
pub fn spawn_io<R, W>(input: R, output: W) -> Receiver<Job>
where
    R: BufRead + Send + 'static,
    W: Write + Send + 'static,
{
    let (job_tx, job_rx) = channel::<Job>();
    let (out_tx, out_rx) = channel::<String>();

    // stdout 线程：只负责写与 flush
    std::thread::Builder::new()
        .name("y901-mcp-out".into())
        .spawn(move || {
            let mut output = output;
            while let Ok(line) = out_rx.recv() {
                if writeln!(output, "{line}").is_err() {
                    break;
                }
                if output.flush().is_err() {
                    break;
                }
            }
        })
        .ok();

    // stdin 线程：读一行 → 投递 → 等 UI 回复 → 转发给 stdout 线程
    std::thread::Builder::new()
        .name("y901-mcp-in".into())
        .spawn(move || {
            let mut input = input;
            for line in input.by_ref().lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                if line.trim().is_empty() {
                    continue;
                }
                let (reply_tx, reply_rx) = channel();
                if job_tx
                    .send(Job {
                        line,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    break;
                }
                // 这里阻塞是**设计**：一次只处理一条，保证响应顺序
                match reply_rx.recv() {
                    Ok(Some(response)) => {
                        if out_tx.send(response).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {} // 通知：无响应
                    Err(_) => break,
                }
            }
        })
        .ok();

    job_rx
}

/// UI 线程：非阻塞地处理一批请求。返回处理条数。
pub fn pump(
    server: &mut McpServer,
    tree: &mut UiTree,
    jobs: &Receiver<Job>,
    budget: usize,
) -> usize {
    let mut done = 0;
    while done < budget {
        let Ok(job) = jobs.try_recv() else {
            break;
        };
        let response = server.handle_line(tree, &job.line);
        // 回传失败说明客户端已断开，忽略即可
        let _ = job.reply.send(response);
        done += 1;
    }
    done
}

/// 无窗口模式：阻塞式主循环（`y901-mcp` 二进制用）。
pub fn run_blocking(server: &mut McpServer, tree: &mut UiTree, jobs: &Receiver<Job>) {
    while let Ok(job) = jobs.recv() {
        let response = server.handle_line(tree, &job.line);
        if job.reply.send(response).is_err() {
            break;
        }
    }
}
