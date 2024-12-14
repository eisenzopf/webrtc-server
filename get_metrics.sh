# Get current metrics
curl http://localhost:8081/api/monitoring/metrics | jq

# Get active alerts
curl http://localhost:8081/api/monitoring/alerts | jq

# Monitor connection success rate
watch -n 5 'curl -s http://localhost:8081/api/monitoring/metrics | jq .success_rate'
