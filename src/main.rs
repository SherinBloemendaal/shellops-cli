fn main() {
    let code = match shellops::cli::run() {
        Ok(code) => code,
        Err(err) => {
            shellops::ui::report_error(&err);
            1
        }
    };
    std::process::exit(code);
}
