use chrono::Local;
use log::LevelFilter;
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
};

/// Directory the log files are written to.
const LOG_DIR: &str = "log";

pub fn initialize() -> Result<(), Box<dyn std::error::Error>> {
    // `FileAppender` does not create the directory for us.
    std::fs::create_dir_all(LOG_DIR)?;

    let logfile = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new(
            "[{d(%Y-%m-%d %H:%M:%S)}] - [{l}] - {m}\n",
        )))
        .build(format!(
            "{}/{}.log",
            LOG_DIR,
            Local::now().format("%Y-%m-%d")
        ))?;

    let config = Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .build(Root::builder().appender("logfile").build(LevelFilter::Info))?;

    log4rs::init_config(config)?;

    Ok(())
}
