## Configuration

The server can be configured using environment variables. You can either set them directly
in your environment or use a `.env` file.

### Required Configuration:
- `STUN_SERVER`: IP address of the STUN server
- `STUN_PORT`: Port for STUN server (default: 3478)
- `TURN_SERVER`: IP address of the TURN server
- `TURN_PORT`: Port for TURN server (default: 3478)
- `TURN_USERNAME`: Username for TURN authentication
- `TURN_PASSWORD`: Password for TURN authentication
- `WS_PORT`: WebSocket port for signaling (default: 8080)

### Optional Configuration:
- `RECORDING_PATH`: Path to store recordings
- `SIP_ENABLED`: Enable SIP integration (true/false)
- `SIP_BIND_ADDRESS`: SIP server bind address
- `SIP_PORT`: SIP server port
- `SIP_DOMAIN`: SIP domain
- `SIP_REALM`: SIP realm

For development, copy `config.env.example` to `.env` and modify as needed:
