/// Rust panic notification only. Never format PanicHookInfo: messages and source
/// locations can contain credentials, repository contents or private paths.
/// Recording is best effort; native crashes and aborts are not captured here.
pub fn install() {
    std::panic::set_hook(Box::new(|_| crate::diagnostics::rust_panic()));
}
