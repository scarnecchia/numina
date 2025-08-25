# Using External SurrealDB

Pattern supports connecting to an external SurrealDB instance instead of the embedded database. This is useful for production deployments or when you need to share data across multiple Pattern instances.

## Configuration

### 1. Enable the Feature

When building pattern-cli, enable the `surreal-remote` feature:

```bash
cargo build --release --bin pattern-cli --features surreal-remote
```

### 2. Configure the Connection

In your `pattern.toml` configuration file, replace the embedded database config with a remote configuration:

```toml
[database]
# Instead of:
# path = "./pattern.db"

# Use:
url = "ws://localhost:8000"  # or "wss://..." for secure connections
namespace = "pattern"
database = "pattern"
# Optional: specify credentials directly in config
# username = "root"
# password = "root"
```

### 3. Environment Variables

For security, you can provide database credentials through environment variables instead of hardcoding them in the config file:

```bash
export SURREAL_USER="root"
export SURREAL_PASS="your-secure-password"
```

These environment variables will be used if `username` and `password` are not specified in the configuration file.

## Running SurrealDB

To start a SurrealDB server for development:

```bash
# Install SurrealDB
curl -sSf https://install.surrealdb.com | sh

# Start with authentication
surreal start --user root --pass root --bind 0.0.0.0:8000 file://./data.db

# Or start in memory mode for testing
surreal start --user root --pass root --bind 0.0.0.0:8000 memory
```

## Migration from Embedded

When migrating from an embedded database to an external one:

1. Export your data using the CAR export feature (format v2):
   ```bash
   pattern-cli export constellation -o backup.car
   ```
   Notes:
   - Export format version is 2 (slim agent metadata + chunked blocks, 1MB cap)
   - Older CARs from pre-v2 builds may not be compatible with all IPLD tools

2. Update your configuration to use the external database

3. Import your data:
   ```bash
   pattern-cli import backup.car
   ```

## Security Considerations

- Never commit credentials to version control
- Use environment variables for production deployments
- Consider using TLS (wss://) for production connections
- Restrict database access to trusted networks only
