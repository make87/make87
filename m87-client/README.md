# m87 Client

Command-line interface and agent for the **make87** platform — enabling secure remote access, monitoring, and container orchestration for edge systems anywhere.

> **Note:** This is the client component. For installation instructions, see the [main README](../README.md).

## Features

- **Agent Management** – Run or install the background agent daemon that connects to the make87 backend
- **Application Management** – Build, push, and run containerized applications
- **Stack Management** – Pull and run versioned Docker Compose files that you define on make87
- **Authentication** – Log in, register, and manage credentials
- **Self-Update** – Seamlessly update the CLI to the latest release

## Usage

### Agent Commands

The agent runs as a background daemon connecting to the make87 backend via WebSocket to sync instructions, logs, and updates.

```bash
# Run the agent interactively
m87 agent run

# Run the agent in headless mode (non-interactive)
m87 agent run --headless

# Install the agent as a system service
m87 agent install

# Check service status
m87 agent status

# Uninstall the agent service
m87 agent uninstall
```

Optional flags for `run` and `install`:

```bash
--user-email <email>
--organization-id <org_id>
```

### Application Commands

```bash
# Build an application (defaults to current directory)
m87 app build [path]

# Push an application to the registry
m87 app push <name> [--version <version>]

# Run an application with optional args
m87 app run <name> [-- args...]
```

### Stack Commands

```bash
# Pull a compose by reference (name:version)
m87 stack pull <name>:<version>

# Run and watch a compose (apply updates)
m87 stack watch <name>
```

### Authentication Commands

```bash
# Log in via OAuth or stored credentials
m87 auth login

# Register a new user or organization
m87 auth register [--user-email <email>] [--organization-id <org_id>]

# Show authentication status
m87 auth status

# Log out and clear credentials
m87 auth logout
```

### Other Commands

```bash
# Update m87 to the latest version
m87 update

# Show version info
m87 version
```

## Architecture

Modules overview:

- **agent** – Agent runtime and system service logic
- **app** – Application build/push/run handling
- **stack** – Stack synchronization and watcher
- **auth** – Login, registration, and token management
- **update** – Self-update logic
- **config** – Config file management
- **server / util** – Shared backend and helper utilities

## Configuration

Configuration is stored in:

- **Linux/macOS**: `~/.config/m87/config.json`
- **Windows**: `%APPDATA%\m87\config.json`

Example:

```json
{
  "api_url": "https://api.make87.com",
  "node_id": null,
  "log_level": "info"
}
```

## Development

For build and test instructions, see the [main README](../README.md#development).

To run the client locally:

```bash
cargo run -p m87-client -- [command]
```
