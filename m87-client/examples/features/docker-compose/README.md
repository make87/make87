# Docker Compose with m87

Deploy Docker Compose projects to remote devices using m87.

## Overview

The `m87 <device> docker` command allows you to run Docker commands on remote devices by using the remote's `DOCKER_HOST`.

## Prerequisites

- Docker CLI installed locally on managing device
- Remote device connected to m87 with Docker installed
- Device user must be in the [`docker` group](https://docs.docker.com/engine/install/linux-postinstall/#manage-docker-as-a-non-root-user) (or have root permissions)

## Basic Usage

All standard Docker commands work through m87:

```bash
# Check Docker version on remote device
m87 <device> docker version

# List containers
m87 <device> docker ps

# Pull an image
m87 <device> docker pull nginx

# Run a container
m87 <device> docker run -d -p 80:80 nginx

# Kill all running containers
m87 <device> docker ps -q | xargs -r m87 <device> docker kill
```

## Docker Compose

Deploy multi-container applications with Docker Compose:

```bash
# Deploy from a local directory with docker-compose.yml to a remote device (includes building on remote device)
cd my-project
m87 <device> docker compose up -d

# Or specify the compose file location
m87 <device> docker compose --project-directory ./my-app up -d

# View logs
m87 <device> docker compose logs -f

# Stop the stack
m87 <device> docker compose down
```

## Example: Simple Compose Project

See [simple/](./simple/) for a working example with:
- Pre-built image from Docker Hub
- Custom service with local Dockerfile
- Environment variables
- Build context handling

Deploy it:
```bash
cd examples/features/docker-compose/simple
m87 <device> docker compose up -d
```

Monitor logs:
```bash
m87 <device> docker compose logs -f
```

Tear it down:
```bash
m87 <device> docker compose down
```
