use std::io::Write;

use chrono::Local;
use env_logger::Builder;

pub fn init() {
    Builder::new()
        .format(|buf, record| {
            writeln!(
                buf,
                "{}\t[{}] [{}:{}]\t{}",
                record.level(),
                Local::now().format("%Y-%m-%d %H:%M:%S.%f"),
                record.target(),
                record.line().unwrap_or(1),
                record.args()
            )
        })
        .parse_default_env()
        .init();
}
