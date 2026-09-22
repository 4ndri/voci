#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    voci::cli::run().await
}
