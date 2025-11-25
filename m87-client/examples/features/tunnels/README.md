# Tunnels

Create secure tunnels to access services on remote devices.

## Overview

m87 tunnels allow you to forward ports from remote devices to your local machine, making remote services accessible as if they were running locally.

## Basic Usage

```bash
# Forward remote port 8080 to same local port (8080)
m87 <device> tunnel 8080

# Forward remote port 8080 to local port 3000
m87 <device> tunnel 8080 3000
```

You can now access the service its mapped port, e.g., `localhost:3000`.

## Common Use Cases

- Access web UIs running on remote devices
- Connect to databases on remote servers
- Debug remote applications locally
- Access admin panels

## Examples

### Access Remote Web Server
```bash
# Remote device running nginx on port 80
m87 rpi tunnel 80 8080

# Access at http://localhost:8080
```

### Connect to Remote Database
```bash
# Remote PostgreSQL on port 5432
m87 db-server tunnel 5432 5432

# Connect locally
psql -h localhost -p 5432 -U myuser mydb
```

### Multiple Tunnels
```bash
# Terminal 1: Forward web UI
m87 device tunnel 3000 3000

# Terminal 2: Forward API
m87 device tunnel 8000 8000
```

## Security

- All traffic is encrypted through the m87 tunnel
- No need to expose ports publicly
- Authentication handled by m87

## Advanced

For persistent tunnels or more complex scenarios, see the [use-cases/](../../use-cases/) directory.