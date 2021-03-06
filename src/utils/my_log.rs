extern crate log;
extern crate time;
extern crate colored;
use colored::*;

use log::{Log, Level,Metadata,Record,SetLoggerError};

struct SimpleLogger{
    level :Level
}

impl Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let level_str = record.level().to_string();
            #[cfg(not(target_os = "windows"))]
            let level_str = {
                match record.level() {
                    Level::Error => level_str.red(),
                    Level::Warn  => level_str.yellow(),
                    Level::Info  => level_str.white(),
                    Level::Debug => level_str.purple(),
                    Level::Trace => level_str.normal(),
                }
            };
            let target = if record.target().len() > 0 {
                record.target()
            } else {
                record.module_path().unwrap_or_default()
            };
            println!( "{} [{:<5}] [{}] {}",
//                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                time::now().strftime("%Y%m%d %T").unwrap().to_string(),
                level_str,
                target,
                record.args());
        }
    }

    fn flush(&self) {
    }
}

pub fn init_with_level(level: Level) -> Result<(), SetLoggerError> {
    log::set_boxed_logger(Box::new(SimpleLogger{ level }))?;
    log::set_max_level(level.to_level_filter());
    Ok(())
}

#[allow(dead_code)]
pub fn init() -> Result<(), SetLoggerError> {
    init_with_level(Level::Trace)
}
