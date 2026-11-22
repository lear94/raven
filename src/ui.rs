use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::IsTerminal;
use std::time::Duration;

pub fn print_banner() {
    if !std::io::stdout().is_terminal() {
        return;
    }
    println!();
    println!(
        "   {}  {}",
        style("RAVEN").red().bold(),
        style(concat!("v", env!("CARGO_PKG_VERSION"))).dim()
    );
    println!("   {}", style("High Performance Package Manager").white());
    println!();
}

pub fn create_download_bar(total_size: u64, msg: &str) -> ProgressBar {
    let pb = if total_size > 0 {
        let pb = ProgressBar::new(total_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{msg}\n{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("#>-"),
        );
        pb
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{msg} {spinner:.green} {bytes} ({bytes_per_sec})")
                .unwrap_or_else(|_| ProgressStyle::default_spinner()),
        );
        pb
    };
    pb.set_message(format!("Downloading {}", style(msg).cyan()));
    pb
}

pub fn create_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
            .template("{spinner:.blue} {msg} [{elapsed}]")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn log_error(msg: &str) {
    eprintln!("{} {}", style("✖ Error:").for_stderr().red().bold(), msg);
}

pub fn log_warn(msg: &str) {
    eprintln!(
        "{} {}",
        style("⚠ Warning:").for_stderr().yellow().bold(),
        msg
    );
}

pub fn log_info(msg: &str) {
    println!("{} {}", style("➜").blue().bold(), msg);
}

pub fn log_success(msg: &str) {
    println!("{} {}", style("✔ Success:").green().bold(), msg);
}
