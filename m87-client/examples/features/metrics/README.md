# System Metrics

Real-time system metrics dashboard for remote devices.

## Overview

`m87 <device> metrics` displays a TUI dashboard with live system metrics from the remote device.

## Basic Usage

```bash
m87 <device> metrics
```

## Dashboard Metrics

| Metric | Description |
|--------|-------------|
| System | Hostname, OS, architecture, uptime, overall CPU % |
| CPU | Per-core usage sparklines with percentages |
| Memory | Usage gauge and history sparkline |
| Disk | Usage gauge and history sparkline |
| Network | Interface table (RX/TX bytes) and throughput sparklines (Mbps) |
| GPU | Memory usage and utilization (if available) |

## Examples

```bash
# Monitor a Raspberry Pi
m87 rpi metrics

# Monitor a server
m87 db-server metrics
```

## Controls

| Key | Action |
|-----|--------|
| `q` | Quit |
| `Ctrl+C` | Quit |

## Use Cases

### Quick Health Check
```bash
m87 rpi metrics
# Glance at CPU, memory, disk usage
# Press q to exit
```

### Debugging Performance Issues
```bash
# Check if device is resource-constrained
m87 edge-node metrics
```

### Monitoring During Deployment
```bash
# Terminal 1: Watch metrics
m87 rpi metrics

# Terminal 2: Deploy application
m87 sync ./app rpi:/home/pi/myapp
m87 rpi exec -- 'cd /home/pi/myapp && npm install'
```

## See Also

- [shell/](../shell/) - Interactive shell for detailed inspection
- [exec/](../exec/) - Run diagnostic commands
