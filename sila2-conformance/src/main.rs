//! Plaintext communication-tester Feature Provider.

use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:50053".to_owned());
    sila2_conformance::serve(address.parse()?).await?;
    Ok(())
}
