#[tokio::main]
async fn main() -> anyhow::Result<()> {
    msg_gateway::run().await
}
