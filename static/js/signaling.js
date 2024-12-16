// signaling.js
import { peerConnection, setupPeerConnection, cleanupExistingConnection, setRemotePeerId } from './webrtc.js';
import { 
    updateStatus, 
    showCallAlert, 
    handlePeerListMessage, 
    updateButtonStates, 
    updateCallStatus,
    updatePeerCheckboxState,
    resetAllPeerCheckboxes 
} from './ui.js';

let ws = null;
let reconnectAttempts = 0;
const MAX_RECONNECT_ATTEMPTS = 5;
let isDisconnecting = false;
let pendingIceCandidates = [];

export async function connect() {
    try {
        if (ws && ws.readyState === WebSocket.OPEN) {
            console.log('WebSocket already connected');
            return;
        }

        // Generate random peer ID if not set
        const peerIdInput = document.getElementById('peerId');
        if (!peerIdInput.value) {
            peerIdInput.value = 'peer_' + Math.random().toString(36).substr(2, 9);
            console.log('Generated peer ID:', peerIdInput.value);
        }

        const serverAddress = window.location.hostname;
        const serverPort = window.location.port || '8080';
        console.log(`Attempting connection to WebSocket server: ws://${serverAddress}:${serverPort}`);
        
        ws = new WebSocket(`ws://${serverAddress}:${serverPort}`);
        
        ws.onopen = () => {
            console.log('WebSocket connection established');
            // Send join message once connected
            const joinMessage = {
                message_type: 'Join',
                room_id: document.getElementById('roomId').value,
                peer_id: document.getElementById('peerId').value
            };
            ws.send(JSON.stringify(joinMessage));
            updateStatus('Connected to signaling server');
            updateButtonStates('connected', 'idle');
        };

        ws.onmessage = async (event) => {
            try {
                const message = JSON.parse(event.data);
                console.log('Received message:', message);

                switch (message.message_type) {
                    case 'PeerList':
                        handlePeerListMessage(message);
                        break;
                    case 'CallRequest':
                        await handleCallRequest(message);
                        break;
                    case 'CallResponse':
                        await handleCallResponse(message);
                        break;
                    case 'IceCandidate':
                        await handleIceCandidate(message);
                        break;
                    case 'EndCall':
                        await handleEndCall(message);
                        break;
                    default:
                        console.warn('Unknown message type:', message.message_type);
                }
            } catch (err) {
                console.error('Error handling message:', err);
                updateStatus('Error handling message: ' + err.message, true);
            }
        };

        ws.onclose = (event) => {
            console.log('WebSocket connection closed:', event);
            updateStatus('Disconnected from signaling server');
            updateButtonStates('disconnected', 'idle');
            
            // Only attempt reconnect if not intentionally disconnecting
            if (!isDisconnecting && reconnectAttempts < MAX_RECONNECT_ATTEMPTS) {
                reconnectAttempts++;
                console.log(`Reconnecting to signaling server (attempt ${reconnectAttempts}/${MAX_RECONNECT_ATTEMPTS})`);
                connect();
            } else {
                console.log('Max reconnection attempts reached, not reconnecting');
            }
        };

        ws.onerror = (error) => {
            console.error('WebSocket error:', error);
            updateStatus('Connection error: ' + error.message, true);
        };
        
    } catch (error) {
        console.error('Connection failed:', error);
        updateStatus('Connection failed: ' + error.message, true);
        updateButtonStates('disconnected', 'idle');
    }
}

export function sendSignal(type, data) {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
        console.error('WebSocket is not connected');
        return;
    }
    ws.send(JSON.stringify({ message_type: type, ...data }));
}

export async function disconnect() {
    if (ws && ws.readyState === WebSocket.OPEN) {
        isDisconnecting = true;
        
        try {
            // First cleanup the WebRTC connection
            console.log('Starting disconnect sequence - cleaning up WebRTC connection...');
            await cleanupExistingConnection();
            
            // Then send disconnect message
            const disconnectMessage = {
                message_type: 'Disconnect',
                room_id: document.getElementById('roomId').value,
                peer_id: document.getElementById('peerId').value
            };
            
            console.log('Sending disconnect message:', disconnectMessage);
            
            // Send disconnect message and wait for confirmation
            await new Promise((resolve, reject) => {
                ws.send(JSON.stringify(disconnectMessage));
                
                // Add a longer timeout to ensure message is sent
                setTimeout(resolve, 500);
            });
            
            console.log('Disconnect message sent successfully');
            
        } catch (err) {
            console.error('Error during disconnect sequence:', err);
            updateStatus('Disconnect error: ' + err.message, true);
        } finally {
            // Close websocket connection last
            console.log('Closing WebSocket connection...');
            if (ws) {
                ws.close();
            }
            updateStatus('Disconnected');
        }
    } else {
        console.log('WebSocket not connected, skipping disconnect sequence');
    }
}

async function handleCallRequest(message) {
    console.log('Received call request:', message);
    const accepted = await showCallAlert(message.from_peer);
    
    if (accepted) {
        try {
            // Setup peer connection before handling any ICE candidates
            await setupPeerConnection();
            setRemotePeerId(message.from_peer);
            
            // Set remote description (offer) first
            await peerConnection.setRemoteDescription(new RTCSessionDescription({
                type: 'offer',
                sdp: message.sdp
            }));
            
            // Create and send answer
            const answer = await peerConnection.createAnswer();
            await peerConnection.setLocalDescription(answer);
            
            // Apply any pending ICE candidates
            while (pendingIceCandidates.length > 0) {
                const candidate = pendingIceCandidates.shift();
                await addIceCandidate(candidate);
            }
            
            const callResponse = {
                room_id: document.getElementById('roomId').value,
                from_peer: document.getElementById('peerId').value,
                to_peer: message.from_peer,
                accepted: true,
                sdp: answer.sdp
            };
            
            sendSignal('CallResponse', callResponse);
            
            updateStatus('Call connected');
            updateCallStatus('connected', message.from_peer);
            updateButtonStates('connected', 'incall');
            // Disable the checkbox for the connected peer
            updatePeerCheckboxState(message.from_peer, true, true);
            
        } catch (err) {
            console.error('Error handling call request:', err);
            updateStatus('Failed to establish call: ' + err.message, true);
            await cleanupExistingConnection();
            resetAllPeerCheckboxes();  // Reset checkboxes on error
        }
    } else {
        // Send rejection response
        sendSignal('CallResponse', {
            room_id: document.getElementById('roomId').value,
            from_peer: document.getElementById('peerId').value,
            to_peer: message.from_peer,
            accepted: false
        });
        // Update UI state
        updateButtonStates('connected', 'idle');
        resetAllPeerCheckboxes();
    }
}

async function handleCallResponse(message) {
    if (message.accepted) {
        console.log('Call accepted, full message:', JSON.stringify(message, null, 2));
        if (!message.sdp) {
            console.error('Call accepted but missing SDP. Full message:', JSON.stringify(message, null, 2));
            updateStatus('Call failed: Missing SDP in response', true);
            await cleanupExistingConnection();
            return;
        }

        try {
            console.log('Setting remote description (answer):', message.sdp.substring(0, 100) + '...');
            await peerConnection.setRemoteDescription(new RTCSessionDescription({
                type: 'answer',
                sdp: message.sdp
            }));
            console.log('Remote description set successfully');
            updateStatus('Call connected');
            updateCallStatus('connected', message.from_peer);
            updateButtonStates('connected', 'incall');
            // Disable the checkbox for the connected peer
            updatePeerCheckboxState(message.from_peer, true, true);
            
            // Apply any pending ICE candidates
            console.log(`Applying ${pendingIceCandidates.length} pending ICE candidates`);
            while (pendingIceCandidates.length > 0) {
                const candidate = pendingIceCandidates.shift();
                await addIceCandidate(candidate);
            }
        } catch (err) {
            console.error('Error setting remote description:', err);
            console.error('Message that caused error:', message);
            updateStatus('Failed to establish call: ' + err.message, true);
            await cleanupExistingConnection();
            updateButtonStates('connected', 'idle');
            resetAllPeerCheckboxes();  // Reset checkboxes on error
        }
    } else {
        console.log('Call rejected by peer:', message.from_peer);
        updateStatus('Call rejected by peer', true);
        await cleanupExistingConnection();
        updateButtonStates('connected', 'idle');
        resetAllPeerCheckboxes();  // Reset checkboxes when call is rejected
    }
}

function handleIceCandidate(message) {
    const candidate = JSON.parse(message.candidate);
    
    if (!peerConnection) {
        console.log('Queuing ICE candidate until peer connection is ready');
        pendingIceCandidates.push(candidate);
        return;
    }
    
    addIceCandidate(candidate);
}

function addIceCandidate(candidate) {
    peerConnection.addIceCandidate(new RTCIceCandidate(candidate))
        .catch(err => console.error('Error adding received ice candidate:', err));
}

function handleConnectionError(message) {
    console.error('Connection error:', message);
    updateStatus(`Connection error: ${message.error}`, true);
    
    if (message.should_retry) {
        console.log('Attempting to reconnect...');
        // Implement reconnection logic here
    }
}

async function handleRemoteCallEnded(peerId) {
    console.log('Remote peer ended call:', peerId);
    try {
        // Clean up WebRTC connection
        await cleanupExistingConnection();
        
        // Additional cleanup for local stream
        if (localStream) {
            localStream.getTracks().forEach(track => track.stop());
            localStream = null;
        }
        
        // Reset video elements
        const localVideo = document.getElementById('localVideo');
        const remoteVideo = document.getElementById('remoteVideo');
        if (localVideo) localVideo.srcObject = null;
        if (remoteVideo) remoteVideo.srcObject = null;
        
        // Update UI
        updateCallStatus('Peer ended call');
        document.getElementById('startCallButton').disabled = false;
        document.getElementById('endCallButton').disabled = true;
        document.getElementById('audioStatus').textContent = 'Audio status: Call ended by peer';
        
    } catch (err) {
        console.error('Error handling remote call end:', err);
        updateStatus('Error handling remote call end: ' + err.message, true);
    }
}

// ... rest of the signaling code ...