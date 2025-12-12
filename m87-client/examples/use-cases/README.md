# m87 CLI Demo Use Cases

Short, focused demos (1-5 min each) for Robotics / Edge AI / Computer Vision community.

---

## Quick Wins (1-2 min)

### Deploy to Fleet in 30 Seconds
Show how fast you can push a new model to edge devices.
```bash
m87 devices                                    # Show fleet of RPis/Jetsons
m87 jetson-01 docker pull mymodel:v2           # Pull new image
m87 jetson-01 docker run -d mymodel:v2         # Deploy
```

### Debug a Remote Robot from Your Couch
No SSH keys, no VPN, no port forwarding.
```bash
m87 robot-arm shell                            # Interactive terminal
htop                                           # Check resources
nvidia-smi                                     # GPU status
tail -f /var/log/robot.log                     # Live logs
```

### Live Camera Feed from Edge Device
View inference output from anywhere.
```bash
m87 jetson-01 tunnel 8080                      # Forward MJPEG stream
# Open localhost:8080 in browser
```

---

## Device Management (2-3 min)

### Zero to Inference: First Device Setup
Fresh device to working inference in under 5 minutes.
```bash
# On device: curl -sSL https://get.make87.com | bash
m87 devices                                    # Shows pending device
m87 approve jetson-nano-01                     # Approve it
m87 jetson-nano-01 docker run -d my-cv-model   # Deploy model
```

### Fleet Overview
```bash
m87 devices                                    # List all devices
m87 devices --online                           # Only online devices
```

---

## Remote Execution (2-3 min)

### Run Commands on Edge Devices
```bash
m87 robot exec -- ls -la /data/captures        # List captured images
m87 robot exec -- df -h                        # Check disk space
m87 robot exec -- cat /proc/cpuinfo            # System info
```

### Interactive TUI Applications
```bash
m87 jetson exec -it -- htop                    # Process monitor
m87 jetson exec -it -- nvtop                   # GPU monitor
m87 jetson exec -it -- vim /etc/config.yaml    # Edit config
```

### Monitor GPU Inference Across Fleet
```bash
m87 jetson-01 exec -- nvidia-smi --query-gpu=utilization.gpu --format=csv -l 1
```

---

## Docker on Edge (3-5 min)

### Docker Compose on Edge Devices
Multi-container CV pipeline.
```bash
m87 jetson-01 docker compose up -d             # Start pipeline
m87 jetson-01 docker ps                        # Check containers
m87 jetson-01 docker logs inference -f         # Follow logs
```

### Container Debugging
```bash
m87 robot docker exec -it myapp /bin/bash      # Shell into container
m87 robot docker stats                         # Resource usage
m87 robot docker logs --tail 100 detector      # Recent logs
```

---

## File Transfer (2-3 min)

### Copy Training Data from Edge
```bash
m87 robot-01 copy /data/captures ./local-captures
```

### Sync Model to Device
```bash
m87 jetson-01 sync ./models /opt/models
```

### Backup Robot Logs
```bash
m87 robot copy /var/log/robot ./robot-logs-backup
```

---

## Port Tunneling - ROS / ROS2 (3-5 min)

### Access ROS2 Topics from Home
DDS discovery over the internet.
```bash
m87 robot tunnel 7400:7500/udp                 # DDS discovery ports
ros2 topic list                                # Works locally now
ros2 topic echo /camera/image_raw              # Subscribe remotely
```

### Foxglove Studio to Remote Robot
Full 3D visualization remotely.
```bash
m87 robot tunnel 8765                          # Foxglove bridge
# Open Foxglove, connect to ws://localhost:8765
```

### Gazebo Simulation Streaming
```bash
m87 workstation tunnel 11345                   # Gazebo port
# Connect local gzclient to remote simulation
```

---

## Port Tunneling - Camera / Video (2-3 min)

### RTSP Camera Through Firewall
```bash
m87 edge-box tunnel 554                        # RTSP port
ffplay -rtsp_transport tcp -i rtsp://localhost:554/stream                # View locally - force TCP to avoid dynamic UDP port usage
```

### Rerun.io Remote Visualization
3D point clouds and sensor data.
```bash
m87 jetson tunnel 9876                         # Rerun server
# Open Rerun viewer, connect to localhost:9876
```

### Isaac Sim Streaming
```bash
m87 workstation tunnel 48010                   # Omniverse streaming
# Stream Isaac Sim viewport to laptop
```

---

## Port Tunneling - ML / Inference (3-5 min)

### Remote TensorBoard
Monitor training on edge GPU.
```bash
m87 jetson tunnel 6006                         # TensorBoard
# Open localhost:6006 in browser
```

### Triton Inference Server
```bash
m87 jetson tunnel 8000,8001,8002               # Triton ports
# Query inference API from local environment
curl localhost:8000/v2/health/ready
```

### Jupyter on Edge Device
Develop directly on device GPU.
```bash
m87 jetson tunnel 8888                         # Jupyter
# Open localhost:8888, use GPU directly
```

### Label Studio on Edge
Label data where it's captured.
```bash
m87 robot tunnel 8080                          # Label Studio
# Annotate images without downloading them
```

### MLflow Tracking
```bash
m87 edge-server tunnel 5000                    # MLflow
# Access experiment tracking UI
```

---

## Port Tunneling - Telemetry / Monitoring (2-3 min)

### Grafana Dashboard for Robot
```bash
m87 robot tunnel 3000                          # Grafana
# View robot dashboards locally
```

### Prometheus Metrics
```bash
m87 robot tunnel 9090                          # Prometheus
# Scrape metrics into central system
```

### Redis State Inspection
Debug robot state machine.
```bash
m87 robot tunnel 6379                          # Redis
redis-cli
> HGETALL robot:state
```

### InfluxDB Telemetry
```bash
m87 robot tunnel 8086                          # InfluxDB
# Query time-series sensor data
```

---

## Port Tunneling - Hardware / Industrial (3-5 min)

### MQTT Broker Access
```bash
m87 edge-device tunnel 1883                    # MQTT
# Connect MQTT Explorer to remote broker
```

### Modbus/PLC Debugging
Access industrial equipment through edge gateway.
```bash
m87 factory-edge tunnel 502                    # Modbus TCP
# Debug PLC from engineering workstation
```

### CAN Bus over IP
```bash
m87 robot tunnel 29536                         # cannelloni
# Bridge remote CAN bus to local analysis tools
```

### OPC-UA Access
```bash
m87 factory-edge tunnel 4840                   # OPC-UA
# Connect UA Expert to remote PLC
```

---

## Serial Port Forwarding (3-5 min)

### Arduino/Microcontroller Debugging
```bash
m87 robot tunnel serial:/dev/ttyUSB0           # Forward serial
# Open local serial monitor (Arduino IDE, PlatformIO)
# Debug firmware remotely
```

### GPS Module Access
```bash
m87 drone tunnel serial:/dev/ttyACM0 --baud 9600
# Read NMEA sentences locally
```

### LIDAR Serial Interface
```bash
m87 robot tunnel serial:/dev/ttyUSB1 --baud 115200
# Configure LIDAR from laptop
```

---

## Development / Debugging (2-3 min)

### VS Code Remote via Tunnel
```bash
m87 jetson tunnel 22                           # SSH port
# VS Code Remote SSH to localhost:22
```

### Remote GDB Debugging
```bash
m87 robot tunnel 3333                          # GDB server
# Attach local GDB to remote process
```

### Remote Docker Build
```bash
m87 jetson tunnel 2375                         # Docker daemon
export DOCKER_HOST=tcp://localhost:2375
docker build -t myapp .                        # Builds on Jetson GPU
```

---

## Priority Matrix

| Demo | Impact | Effort | Target Audience |
|------|--------|--------|-----------------|
| Zero to Inference | High | Low | Everyone |
| ROS2 DDS Tunneling | High | Medium | ROS developers |
| Live Camera Feed | High | Low | CV engineers |
| Docker on Edge | High | Low | ML engineers |
| Serial Forwarding | Medium | Low | Hardware/firmware |
| Foxglove Remote | Medium | Low | ROS developers |
| Triton Server | Medium | Medium | ML inference |
| Fleet Deploy | High | Low | DevOps/MLOps |

---

## Recording Tips

1. **Keep it focused** - One capability per video
2. **Show the problem first** - "Normally you'd need to SSH, set up keys, configure VPN..."
3. **Real devices** - Use actual Jetson/RPi, not simulated
4. **Live terminal** - No cuts, show it's real-time
5. **End with result** - Show the working camera feed, dashboard, etc.
