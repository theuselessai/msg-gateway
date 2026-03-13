#[tokio::main]
async fn main() -> anyhow::Result<()> {
    plit_gw::run().await
}
