#!/bin/sh

echo "Custom service started"
echo "Environment: $TEST_VAR"
echo "Hostname: $(hostname)"

# Keep container running
while true; do
    sleep 3600
done
