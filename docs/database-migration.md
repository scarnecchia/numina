# Database Migration Guide

## Overview

Pattern uses SurrealDB for data persistence, supporting both embedded (local file) and remote (server) configurations. This guide covers migrating from embedded to external SurrealDB instances.

## Database Configuration Types

### Embedded Database (Default)
```toml
[database]
type = "embedded"
path = "./pattern.db"  # Local file storage
strict_mode = false
```

### Remote Database
```toml
[database]
type = "remote"
url = "ws://localhost:8000"  # or "wss://" for TLS
username = "root"
password = "root"
namespace = "pattern"
database = "pattern"
```

## Migration Process: Embedded to External

### Prerequisites
- SurrealDB CLI installed (`curl -sSf https://install.surrealdb.com | sh`)
- Access to both source and target databases
- Sufficient disk space for export file

### Step 1: Export from Embedded Database

Export the entire embedded database to a SurrealQL script:

```bash
surreal export \
  --endpoint surrealkv://pattern.db \
  --namespace pattern \
  --database pattern \
  pattern_export.sql
```

For selective export, use the `--only` flag with specific resources:
```bash
surreal export \
  --endpoint surrealkv://pattern.db \
  --namespace pattern \
  --database pattern \
  --only \
  --tables agent,message,mem \
  --records true \
  pattern_export.sql
```

### Step 2: Start External SurrealDB Server

Start a SurrealDB server instance:

```bash
# Development server (in-memory)
surreal start --log debug --user root --pass root memory

# Production server (with persistent storage)
surreal start \
  --log info \
  --user root \
  --pass <secure-password> \
  file://./surrealdb-data
```

For production deployments, consider using Docker:
```bash
docker run --rm -p 8000:8000 \
  -v $(pwd)/surrealdb-data:/data \
  surrealdb/surrealdb:latest \
  start --user root --pass root file://data
```

### Step 3: Import to External Server

Import the exported data into the external server:

```bash
surreal import \
  --endpoint http://localhost:8000 \
  --username root \
  --password root \
  --namespace pattern \
  --database pattern \
  pattern_export.sql
```

### Step 4: Update Pattern Configuration

Modify your `pattern.toml` or `config.toml`:

```toml
[database]
type = "remote"
url = "ws://localhost:8000"  # Use wss:// for TLS connections
username = "root"
password = "root"  # Use environment variable in production
namespace = "pattern"
database = "pattern"
```

For production, use environment variables:
```toml
[database]
type = "remote"
url = "${SURREALDB_URL}"
username = "${SURREALDB_USER}"
password = "${SURREALDB_PASS}"
namespace = "pattern"
database = "pattern"
```

### Step 5: Verify Migration

Test the connection and verify data integrity:

```bash
# Test with Pattern CLI
pattern-cli agent list

# Or query directly
echo "SELECT * FROM agent;" | surreal sql \
  --endpoint http://localhost:8000 \
  --username root \
  --password root \
  --namespace pattern \
  --database pattern
```

## Multi-Tenant Setup

For hosting multiple Pattern instances on one SurrealDB server:

### Option 1: Namespace Isolation
Each tenant gets their own namespace:
```toml
# Tenant A
[database]
namespace = "tenant_a"
database = "pattern"

# Tenant B  
[database]
namespace = "tenant_b"
database = "pattern"
```

### Option 2: Database Isolation
Each tenant gets their own database within a shared namespace:
```toml
# Tenant A
[database]
namespace = "pattern"
database = "tenant_a"

# Tenant B
[database]
namespace = "pattern"  
database = "tenant_b"
```

## Backup Strategies

### Automated Backups
Create a backup script (`backup-pattern.sh`):
```bash
#!/bin/bash
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BACKUP_FILE="pattern_backup_${TIMESTAMP}.sql"

surreal export \
  --endpoint ${SURREALDB_URL} \
  --username ${SURREALDB_USER} \
  --password ${SURREALDB_PASS} \
  --namespace pattern \
  --database pattern \
  "${BACKUP_FILE}"

# Compress the backup
gzip "${BACKUP_FILE}"

# Optional: Upload to S3/GCS/Azure
# aws s3 cp "${BACKUP_FILE}.gz" s3://my-backups/pattern/
```

Schedule with cron:
```bash
# Daily backup at 2 AM
0 2 * * * /path/to/backup-pattern.sh
```

### Point-in-Time Recovery
For production environments, consider SurrealDB's upcoming features:
- Change Data Capture (CDC)
- Transaction logs
- Continuous replication

## Performance Considerations

### Connection Pooling
Pattern automatically handles connection pooling, but for high-load scenarios:
- Use WebSocket (`ws://` or `wss://`) instead of HTTP
- Deploy SurrealDB behind a load balancer for horizontal scaling
- Consider SurrealDB cluster mode (when available)

### Network Latency
When migrating from embedded to remote:
- Expect ~1-5ms latency for local network
- Expect ~10-50ms latency for cross-region
- Use connection keep-alive for long-running agents
- Consider read replicas for geographically distributed deployments

## Troubleshooting

### Common Issues

1. **Connection Refused**
   - Verify SurrealDB server is running
   - Check firewall rules
   - Ensure correct endpoint URL

2. **Authentication Failed**
   - Verify username/password
   - Check auth-level (root vs namespace vs database)
   - Ensure user has correct permissions

3. **Namespace/Database Not Found**
   - SurrealDB requires explicit namespace/database creation
   - Run migrations after import: `pattern-cli db migrate`

4. **Schema Mismatch**
   - Pattern includes automatic migrations
   - Force schema update: `pattern-cli db migrate --force`

### Debug Commands

```bash
# Test connection
surreal isready --endpoint http://localhost:8000

# View server info
surreal info --endpoint http://localhost:8000 \
  --username root --password root

# Check database structure
echo "INFO FOR DB;" | surreal sql \
  --endpoint http://localhost:8000 \
  --username root --password root \
  --namespace pattern --database pattern
```

## Security Best Practices

1. **Authentication**
   - Never use default credentials in production
   - Use strong, unique passwords
   - Consider JWT tokens for service accounts

2. **Network Security**
   - Always use TLS (`wss://`) for remote connections
   - Restrict database access to specific IPs
   - Use VPN or private networks when possible

3. **Access Control**
   - Create specific users for Pattern (not root)
   - Use SurrealDB's RBAC for fine-grained permissions
   - Audit database access logs regularly

4. **Data Encryption**
   - Enable encryption at rest for SurrealDB storage
   - Use encrypted connections (TLS/SSL)
   - Consider field-level encryption for sensitive data

## References

- [SurrealDB Documentation](https://surrealdb.com/docs)
- [SurrealDB Export/Import Guide](https://surrealdb.com/docs/cli/export)
- [Pattern Database Configuration](./configuration.md#database)
- [SurrealDB Docker Deployment](https://surrealdb.com/docs/installation/running/docker)