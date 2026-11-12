use futures::FutureExt;

#[tokio::main]
async fn main() {
    let _ = color_eyre::install();
    let code = jui::cli::main(|s| jui::tui::run(s).boxed()).await;
    std::process::exit(code);
}
