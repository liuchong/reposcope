use reposcope::{ScopeError, cli};

#[tokio::main]
async fn main() {
    let code = match cli::run().await {
        Ok(()) => 0,
        Err(e @ ScopeError::Usage(_)) => {
            eprintln!("error: {e}");
            1
        }
        Err(e @ ScopeError::Api(_)) => {
            eprintln!("error: {e}");
            2
        }
        Err(e @ ScopeError::Render(_)) => {
            eprintln!("error: {e}");
            1
        }
    };
    std::process::exit(code);
}
