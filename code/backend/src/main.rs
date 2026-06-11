#[tokio::main]
async fn main() -> anyhow::Result<()> {
    kubarr::bootstrapper::run().await
}
