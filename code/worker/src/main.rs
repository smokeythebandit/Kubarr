#[tokio::main]
async fn main() -> anyhow::Result<()> {
    kubarr::runtime::run_worker().await
}
