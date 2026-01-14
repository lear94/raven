use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use std::time::Duration;

pub fn print_banner() {
    println!();
    println!(
        "   {}  {}",
        "RAVEN".red().bold(),
        format!("v{}", env!("CARGO_PKG_VERSION")).dimmed()
    );
    println!("   {}", "High Performance Package Manager".white());
    println!();
}

pub fn create_download_bar(total_size: u64, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(total_size);
    pb.set_style(ProgressStyle::default_bar()
        .template("{msg}\n{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(format!("Downloading {}", msg.cyan()));
    pb
}

pub fn create_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn log_error(msg: &str) {
    println!("{} {}", "✖ Error:".red().bold(), msg);
}

pub fn log_success(msg: &str) {
    println!("{} {}", "✔ Success:".green().bold(), msg);
}
