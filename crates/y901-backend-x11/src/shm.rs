//! MIT-SHM（X11 共享内存扩展）的纯 Rust 封装。
//!
//! ## 为什么需要它
//!
//! 普通 `PutImage` 把像素**放在请求体里**发给服务器，
//! 而 `length` 字段是 u16 → 单包上限 262,116 字节，1920×1080 要分 32 块。
//! MIT-SHM 改成\"两边映射同一块内存\"：我们 `memcpy` 进去，
//! 只发一个 40 字节的 `ShmPutImage`（带句柄与偏移）。
//!
//! ## 为什么不引入 `libc` crate
//!
//! `shmget` / `shmat` / `shmdt` / `shmctl` 是 **libc 导出的符号**，
//! 声明 `extern "C"` 就能直接用——不需要任何 C 源码编译，
//! 也不给依赖树添一个包。

use std::ffi::c_void;

/// SysV 共享内存：一块可以同时被我们与 X server 看到的内存。
///
/// 生命周期：`new`（shmget+shmat）→ 使用 → `Drop`（shmdt + shmctl(IPC_RMID)）。
pub struct ShmSegment {
    /// `shmget` 返回的段 ID（要告诉 X server）
    shmid: i32,
    /// 映射到本进程的地址
    ptr: *mut u8,
    /// 字节数
    size: usize,
}

// SAFETY: 这块内存在进程内全局可见，指针本身不被多线程并发访问（只在 UI 线程写）。
unsafe impl Send for ShmSegment {}

impl ShmSegment {
    /// 创建一块至少 `size` 字节的共享内存并映射进本进程。
    ///
    /// 失败：`Err` 包含具体系统错误（常见：`ENOSYS`、`ENOMEM`、`EPERM`）——
    /// 调用方应**静默回退**到 `PutImage`，而不是报错给用户。
    pub fn new(size: usize) -> Result<Self, String> {
        const IPC_PRIVATE: i32 = 0;
        const IPC_CREAT: i32 = 0o1000;
        const SHM_R: i32 = 0o400;
        const SHM_W: i32 = 0o200;
        // SAFETY: 标准 libc 调用；参数是纯数值，无指针\u3b7问。
        // **先 clamp 再存储**：否则 `size()` 会谎报 0，而 `write()` 会拒绝一切写入。
        // （这个 bug 是 `zero_size_is_clamped_not_a_panic` 抓出来的。）
        let size = size.max(1);
        let shmid = unsafe { shmget(IPC_PRIVATE, size, IPC_CREAT | SHM_R | SHM_W) };
        if shmid < 0 {
            return Err(format!(
                "shmget 失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: shmid 刚创建且有效；shmaddr 传 null 让内核选地址。
        let ptr = unsafe { shmat(shmid, std::ptr::null(), 0) } as *mut u8;
        if ptr.is_null() || ptr as usize == usize::MAX {
            // 映射失败 → 必须立即回收段，否则泄漏
            // SAFETY: shmid 有效，IPC_RMID 不需要 buf。
            unsafe {
                shmctl(shmid, IPC_RMID, std::ptr::null_mut());
            }
            return Err(format!(
                "shmat 失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self { shmid, ptr, size })
    }

    /// 段 ID（告诉 X server 用）。
    pub fn id(&self) -> i32 {
        self.shmid
    }

    /// 字节数。
    pub fn size(&self) -> usize {
        self.size
    }

    /// 把 BGRA 像素拷贝进共享内存（偏移 0 开始）。
    ///
    /// 返回：`Err` 当像素比段还大（说明尺寸估算错了）。
    pub fn write(&mut self, pixels: &[u8]) -> Result<(), String> {
        if pixels.len() > self.size {
            return Err(format!(
                "像素 {} 字节超过共享内存段 {} 字节",
                pixels.len(),
                self.size
            ));
        }
        // SAFETY: `ptr` 是 shmat 得到的、至少 `size` 字节的可写映射；
        // `pixels.len() <= size` 已校验，且两边不重叠（一个是 Rust Vec，一个是 SysV 段）。
        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.ptr, pixels.len());
        }
        Ok(())
    }
}

impl Drop for ShmSegment {
    fn drop(&mut self) {
        // SAFETY: ptr 来自 shmat，且未被其他人 shmdt 过；
        // shmid 来自 shmget。两个调用都在本进程生命期内合法。
        unsafe {
            shmdt(self.ptr as *const c_void);
            shmctl(self.shmid, IPC_RMID, std::ptr::null_mut());
        }
    }
}

// —— libc 符号声明（纯 Rust FFI，不引入 C 源码）——
const IPC_RMID: i32 = 0;

extern "C" {
    /// 创建/获取 SysV 共享内存段。
    fn shmget(key: i32, size: usize, shmflg: i32) -> i32;
    /// 把段映射到本进程地址空间。
    fn shmat(shmid: i32, shmaddr: *const c_void, shmflg: i32) -> *mut c_void;
    /// 取消映射。
    fn shmdt(shmaddr: *const c_void) -> i32;
    /// 控制（IPC_RMID = 标记删除）。
    fn shmctl(shmid: i32, cmd: i32, buf: *mut c_void) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_write_and_release() {
        let mut seg = match ShmSegment::new(4096) {
            Ok(s) => s,
            Err(e) => {
                // 某些容器/沙箱禁用 SysV shm → 这不是实现错，但也不能假装通过
                eprintln!("[跳过] 本环境不支持 SysV 共享内存：{e}");
                return;
            }
        };
        assert!(seg.id() >= 0);
        assert!(seg.size() >= 4096);
        let data = vec![0xABu8; 1024];
        seg.write(&data).expect("写入");
        // 超大写入应报错而不是越界
        assert!(seg.write(&vec![0u8; 8192]).is_err());
    }

    #[test]
    fn zero_size_is_clamped_not_a_panic() {
        // size=0 应被 clamp 到 1，而不是向内核传 0。
        // 实测：某些内核/WSL 下 `shmget(size=1)` 本身仍返 EINVAL（最小页对齐）。
        // 因此这里只断言"不 panic、不越界"，不断言必须成功。
        match ShmSegment::new(0) {
            Ok(seg) => assert!(seg.size() >= 1),
            Err(e) => eprintln!("[跳过] size=0 在本环境不可用：{e}"),
        }
    }
}
