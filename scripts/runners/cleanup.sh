#!/bin/bash
# Daily cleanup to prevent ZFS pool from filling up

set -e
LOG="/mnt/storage/personal/runners/cleanup.log"
echo "[$(date -Iseconds)] Starting CI cleanup" >> "$LOG"

# 1. Remove stopped containers, dangling images, unused networks, build cache
docker system prune -f >> "$LOG" 2>&1

# 2. Remove ALL unused Docker images (including cached build images)
#    Safe because runners are ephemeral and re-pull what they need
docker image prune -af >> "$LOG" 2>&1

# 3. Remove unused Docker volumes (catches leftover Kind node volumes)
docker volume prune -f >> "$LOG" 2>&1

# 4. Clean orphaned Kind clusters (e2e tests create named clusters;
#    if a job crashes without cleanup, they pile up as Docker containers)
for cluster in $(kind get clusters 2>/dev/null | grep -E '^kubarr-e2e-'); do
    echo "Removing orphaned Kind cluster: $cluster" >> "$LOG"
    kind delete cluster --name "$cluster" >> "$LOG" 2>&1
done

# 5. Remove runner work dirs older than 3 days
find /mnt/storage/personal/runners/work -mindepth 1 -maxdepth 1 \
     -type d -mtime +3 -exec rm -rf {} + >> "$LOG" 2>&1

# 6. Rotate cleanup log (keep last 500 lines)
tail -500 "$LOG" > "${LOG}.tmp" && mv "${LOG}.tmp" "$LOG"

echo "[$(date -Iseconds)] Cleanup done. ZFS usage: $(zfs list personal_data | tail -1 | awk '{print $2"/"$3}')" >> "$LOG"
