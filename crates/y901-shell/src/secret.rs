//! 平台无关的**敏感信息加密**接口。
//!
//! ## 设计原则（宁可失败，不可降级）
//!
//! - **Windows**：DPAPI（`CryptProtectData`）—— 系统凭据级，
//!   同机同用户可解密，换机器就解不开（这是特性，不是缺陷）；
//! - **Linux / macOS**：**不存储密码**，直接返回 `Err`。
//!
//! ## 为什么 Linux 不降级为明文（或自己造一个加密）
//!
//! 1. **明文存盘是安全红线**：一旦写进 `config.yaml`，用户的 SSH 密码就在磁盘上
//!    以可读形式存在（备份、同步盘、截图都会泄露）；
//! 2. 自己造一个"用硬编码密钥 XOR"更糟：它给人一种已加密的错觉，实际强度为零；
//! 3. Linux 上有更好的替代方案：**SSH 密钥** 或 **环境变量**（进程环境不落盘）。
//!
//! 所以这里的策略是：**宁可让功能明确失败，也不给一个假的安全感**。

/// 一个可用的密钥存储实现的名字（日志/诊断用）。
pub fn backend_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "DPAPI"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "不支持（本平台不存储密码）"
    }
}

/// 本平台是否支持加密存储密码。
///
/// 用途：UI 可以在 Linux 上提前禁用"记住密码"复选框，
/// 而不是等用户点保存才报错。
pub fn is_supported() -> bool {
    cfg!(target_os = "windows")
}

/// 本平台不支持时给出的统一提示。
#[cfg(not(target_os = "windows"))]
const UNSUPPORTED: &str = "当前平台不支持密码存储（不会以明文保存）。\
请使用 SSH 密钥，或在启动 Helm 前设置环境变量 HELM_SSH_PASSWORD。";

/// 加密一段明文，返回密文字节。
///
/// Windows：DPAPI（用户级，不要求弹窗 —— 进程无人守时不会卡住）。
/// 其他平台：`Err`。副作用：无（不写文件）。
pub fn encrypt(plain: &[u8]) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "windows")]
    {
        imp::protect(plain)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = plain;
        Err(UNSUPPORTED.to_string())
    }
}

/// 解密一段密文，返回明文字节。
///
/// 其他平台：`Err`。副作用：无。
pub fn decrypt(cipher: &[u8]) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "windows")]
    {
        imp::unprotect(cipher)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = cipher;
        Err(UNSUPPORTED.to_string())
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    /// 用 DPAPI 加密。
    ///
    /// 为什么用 `windows-sys` 而不是 `windows`：后者带 RAII 包装（更友好），
    /// 但 `y901-shell` 是跳平台层，只需最小表达；而且 `windows-sys` 已在工作区里。
    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: plain.len() as u32,
            pbData: plain.as_ptr() as *mut u8,
        };
        let mut out_blob = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: 两个 blob 都在本函数栈上生命；DPAPI 不保留输入指针；
        // 作用域参数全传 null（不需“用户确认”弹窗）。
        let ok = unsafe {
            CryptProtectData(
                &in_blob,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 {
            return Err(format!(
                "DPAPI 加密失败（错误码 {}）",
                std::io::Error::last_os_error()
            ));
        }
        Ok(take_blob(&mut out_blob))
    }

    /// 用 DPAPI 解密。
    pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: cipher.len() as u32,
            pbData: cipher.as_ptr() as *mut u8,
        };
        let mut out_blob = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: 同 protect；out_blob 由 DPAPI 分配，必须用 LocalFree 释放。
        let ok = unsafe {
            CryptUnprotectData(
                &in_blob,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };
        if ok == 0 {
            return Err(format!(
                "密码解密失败（可能不是本机本用户加密的，错误码 {}）",
                std::io::Error::last_os_error()
            ));
        }
        Ok(take_blob(&mut out_blob))
    }

    /// 把 DPAPI 返回的 blob 拷贝出来并释放。
    fn take_blob(blob: &mut CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: DPAPI 保证成功时 pbData 指向 cbData 字节可读内存；
        // 拷贝后立即 LocalFree，不让指针逃出本函数。
        let v = unsafe {
            let slice = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize);
            let v = slice.to_vec();
            LocalFree(blob.pbData as *mut core::ffi::c_void);
            v
        };
        blob.pbData = std::ptr::null_mut();
        blob.cbData = 0;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_when_supported() {
        if !is_supported() {
            // 这不是"跳过"，而是断言本平台的**正确行为**：明确报错，不降级
            let e = encrypt(b"secret").expect_err("不支持的平台必须返回 Err");
            assert!(e.contains("HELM_SSH_PASSWORD"), "错误提示要指出替代方案：{e}");
            return;
        }
        let plain = b"sk-1234567890abcdef";
        let cipher = encrypt(plain).expect("加密");
        assert_ne!(cipher.as_slice(), plain, "密文不能等于明文");
        assert_eq!(decrypt(&cipher).expect("解密"), plain);
    }

    #[test]
    fn garbage_ciphertext_is_an_error_not_a_panic() {
        let e = decrypt(b"not-a-dpapi-blob");
        assert!(e.is_err());
    }

    #[test]
    fn backend_name_is_never_empty() {
        assert!(!backend_name().is_empty());
    }
}
