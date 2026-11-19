//! Stand-in for prismlauncher.exe in demos and tests: records how it was called.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let me = std::env::current_exe().unwrap();
    let log = me.parent().unwrap().join("launched.txt");
    std::fs::write(log, args.join(" ")).unwrap();
}
