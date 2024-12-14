// Import shared state and functions from webrtc.js
import {
    peerConnection,
    localStream,
    remotePeerId,
    setupPeerConnection,
    handleTrack,
    getIceServers,
    enableVideo,
    resetPeerConnection,
    resetLocalStream,
    resetRemotePeerId,
    setRemotePeerId,
    setLocalStream,
    isInitiator
} from './webrtc.js';

import { updateStatus, handlePeerListMessage } from './ui.js';

let ws;
let isDisconnecting = false;
let activeCallRequests = new Set();
let reconnectAttempts = 0;
const MAX_RECONNECT_ATTEMPTS = 5;
let iceCandidateQueue = [];
let iceCandidateBuffer = [];
let isNegotiating = false;
let isRemoteDescriptionSet = false;

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
            console.log('WebSocket connected successfully');
            reconnectAttempts = 0;
            handleWebSocketOpen();
        };
        
        ws.onmessage = (event) => {
            console.log('Received WebSocket message:', event.data);
            handleWebSocketMessage(event);
        };
        
        ws.onerror = (error) => {
            console.error('WebSocket error:', error);
            handleWebSocketError(error);
        };
        
        ws.onclose = (event) => {
            console.log('WebSocket closed:', {
                code: event.code,
                reason: event.reason,
                wasClean: event.wasClean
            });
            handleWebSocketClose(event);
            
            if (!isDisconnecting && reconnectAttempts < MAX_RECONNECT_ATTEMPTS) {
                const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), 10000);
                console.log(`Connection lost. Attempting to reconnect in ${delay}ms (${reconnectAttempts + 1}/${MAX_RECONNECT_ATTEMPTS})`);
                reconnectAttempts++;
                setTimeout(connect, delay);
            } else if (reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
                console.error('Max reconnection attempts reached');
                updateStatus('Connection failed - please refresh', true);
            }
        };

    } catch (err) {
        console.error('Error connecting:', err);
        updateStatus('Failed to connect', true);
    }
}

function handleWebSocketOpen() {
    console.log('WebSocket connection established');
    updateStatus('Connected to signaling server');
    const roomId = document.getElementById('roomId').value;
    const peerId = document.getElementById('peerId').value;
    
    // Join room
    sendSignal('Join', {
        room_id: roomId,
        peer_id: peerId
    });
    
    // Request initial peer list
    sendSignal('RequestPeerList', {
        room_id: roomId
    });
}

async function handleWebSocketMessage(event) {
    try {
        const message = JSON.parse(event.data);
        console.log("Received message:", message);

        switch (message.message_type) {
            case "PeerList":
                console.log("Received PeerList message:", message);
                handlePeerListMessage(message);
                break;

            case "CallRequest":
                console.log("Received CallRequest message:", message);
                if (message.to_peers.includes(document.getElementById('peerId').value)) {
                    try {
                        // First check/request permissions
                        await navigator.mediaDevices.getUserMedia({ video: true, audio: true });
                        
                        setRemotePeerId(message.from_peer);
                        updateStatus(`Incoming call from ${remotePeerId}`);
                        
                        // Setup peer connection first
                        if (!peerConnection) {
                            await setupPeerConnection();
                        }

                        // Set remote description from the offer BEFORE sending response
                        const remoteDesc = new RTCSessionDescription({
                            type: 'offer',
                            sdp: message.sdp
                        });
                        
                        await peerConnection.setRemoteDescription(remoteDesc);
                        console.log("Set remote description from offer");

                        // Create and set local answer
                        const answer = await peerConnection.createAnswer();
                        await peerConnection.setLocalDescription(answer);
                        
                        // Only send acceptance after we've set up the connection
                        sendSignal('CallResponse', {
                            room_id: document.getElementById('roomId').value,
                            from_peer: document.getElementById('peerId').value,
                            to_peer: remotePeerId,
                            accepted: true
                        });

                        // Send answer
                        sendSignal('Answer', {
                            room_id: message.room_id,
                            sdp: answer.sdp,
                            from_peer: document.getElementById('peerId').value,
                            to_peer: message.from_peer
                        });

                    } catch (err) {
                        console.error('Error handling call request:', err);
                        sendSignal('CallResponse', {
                            room_id: document.getElementById('roomId').value,
                            from_peer: document.getElementById('peerId').value,
                            to_peer: message.from_peer,
                            accepted: false,
                            reason: 'Failed to setup connection: ' + err.message
                        });
                        updateStatus('Failed to setup connection: ' + err.message, true);
                    }
                }
                break;

            case "CallResponse":
                console.log("Received CallResponse message:", message);
                await handleCallResponseMessage(message);
                break;

            case "Offer":
                await handleOfferMessage(message);
                break;

            case "Answer":
                await handleAnswerMessage(message);
                break;

            case "IceCandidate":
                await handleIceCandidateMessage(message);
                break;

            case "EndCall":
                handleEndCallMessage();
                break;

            default:
                console.log("Unhandled message type:", message.message_type);
        }
    } catch (error) {
        console.error("Error handling WebSocket message:", error);
    }
}

function handleWebSocketError(error) {
    console.error('WebSocket error:', error);
    updateStatus('WebSocket error occurred', true);
}

function handleWebSocketClose(event) {
    console.log('WebSocket connection closed:', event.code, event.reason);
    updateStatus('Connection closed');
    
    if (!isDisconnecting && event.code === 1006) {
        console.log('Attempting to reconnect...');
        setTimeout(() => {
            connectWebSocket();
        }, 2000); // Retry after 2 seconds
    }
    
    cleanupConnection();
}

export function sendSignal(messageType, data) {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
        console.error('WebSocket is not connected');
        return;
    }

    // Special handling for ICE candidates
    if (messageType === 'IceCandidate' && data.candidate) {
        data.candidate = JSON.stringify(data.candidate);
    }

    const message = {
        message_type: messageType,
        ...data
    };

    console.log('Sending message:', message);
    ws.send(JSON.stringify(message));
}

export function disconnect() {
    try {
        isDisconnecting = true;
        cleanupConnection();
        updateStatus('Disconnected');
    } catch (err) {
        console.error('Error during disconnect:', err);
        updateStatus('Disconnect error: ' + err.message, true);
    }
}

function cleanupConnection() {
    if (peerConnection) {
        peerConnection.close();
    }

    if (localStream) {
        localStream.getTracks().forEach(track => track.stop());
    }

    // Remove remote audio element if it exists
    const remoteAudio = document.getElementById('remoteAudio');
    if (remoteAudio) {
        remoteAudio.srcObject = null;
        remoteAudio.remove();
    }

    // Use the reset functions instead of direct assignment
    resetPeerConnection();
    resetLocalStream();
    resetRemotePeerId();
    document.getElementById('audioStatus').textContent = 'Audio status: Not in call';
}

async function handleCallResponseMessage(message) {
    if (message.accepted) {
        setRemotePeerId(message.from_peer);
        updateStatus(`Call accepted by ${remotePeerId}`);
        
        if (!isInitiator) {
            try {
                if (!peerConnection) {
                    await setupPeerConnection();
                }
                console.log('Call accepted, waiting for answer...');
            } catch (err) {
                console.error('Error setting up connection:', err);
                updateStatus('Failed to setup connection: ' + err.message, true);
            }
        }
    } else {
        updateStatus(`Call rejected by ${message.from_peer}`, true);
        await cleanupExistingConnection();
    }
}

async function handleOfferMessage(message) {
    console.log("Received offer from:", message.from_peer);
    setRemotePeerId(message.from_peer);
    
    try {
        // 1. Setup peer connection if not exists
        if (!peerConnection) {
            await setupPeerConnection();
        }

        // 2. Set remote description from offer
        const remoteDesc = new RTCSessionDescription({
            type: 'offer',
            sdp: message.sdp
        });
        
        await peerConnection.setRemoteDescription(remoteDesc);
        console.log("Set remote description from offer");

        // 3. Create and set local description (answer)
        const answer = await peerConnection.createAnswer();
        await peerConnection.setLocalDescription(answer);
        console.log("Created and set local answer");

        // 4. Send answer
        sendSignal('Answer', {
            room_id: message.room_id,
            sdp: answer.sdp,
            from_peer: document.getElementById('peerId').value,
            to_peer: message.from_peer
        });

        // 5. Process any buffered candidates
        console.log(`Processing ${iceCandidateBuffer.length} buffered ICE candidates`);
        while (iceCandidateBuffer.length > 0) {
            const candidate = iceCandidateBuffer.shift();
            try {
                await peerConnection.addIceCandidate(candidate);
                console.log('Added buffered ICE candidate');
            } catch (err) {
                console.warn('Error adding buffered candidate:', err);
            }
        }

    } catch (err) {
        console.error('Error handling offer:', err);
        updateStatus('Failed to handle offer: ' + err.message, true);
    }
}

async function handleAnswerMessage(message) {
    console.log("Received answer from:", message.from_peer, "Current signaling state:", peerConnection?.signalingState);
    
    try {
        if (!peerConnection) {
            throw new Error("No peer connection exists");
        }

        const remoteDesc = new RTCSessionDescription({
            type: 'answer',
            sdp: message.sdp
        });

        await peerConnection.setRemoteDescription(remoteDesc);
        console.log("Set remote description from answer, new signaling state:", peerConnection.signalingState);

        // Process any buffered candidates
        console.log(`Processing ${iceCandidateBuffer.length} buffered ICE candidates`);
        while (iceCandidateBuffer.length > 0) {
            const candidate = iceCandidateBuffer.shift();
            try {
                await peerConnection.addIceCandidate(candidate);
                console.log('Added buffered ICE candidate');
            } catch (err) {
                console.warn('Error adding buffered candidate:', err);
            }
        }

    } catch (err) {
        console.error('Error handling answer:', err);
        updateStatus('Failed to handle answer: ' + err.message, true);
    }
}

async function handleIceCandidateMessage(message) {
    try {
        console.log('Received ICE candidate message:', message);
        
        if (!peerConnection) {
            console.warn('Received ICE candidate but no peer connection exists');
            iceCandidateBuffer.push(message.candidate);
            return;
        }

        // Parse the candidate if it's a string
        const candidate = typeof message.candidate === 'string' 
            ? JSON.parse(message.candidate) 
            : message.candidate;

        try {
            if (peerConnection.remoteDescription && peerConnection.remoteDescription.type) {
                await peerConnection.addIceCandidate(new RTCIceCandidate(candidate));
                console.log('Added ICE candidate immediately');
            } else {
                console.log('Buffering ICE candidate until remote description is set');
                iceCandidateBuffer.push(candidate);
            }
        } catch (err) {
            console.error('Error adding ICE candidate:', err);
            iceCandidateBuffer.push(candidate);
        }
    } catch (err) {
        console.error('Error handling ICE candidate:', err);
        console.error('Candidate that caused error:', message.candidate);
    }
}