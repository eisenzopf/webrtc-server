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
    setLocalStream
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

        const serverAddress = document.getElementById('stunServer').value;
        console.log(`Attempting connection to WebSocket server: ws://${serverAddress}:8080`);
        
        ws = new WebSocket(`ws://${serverAddress}:8080`);
        
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
        if (typeof event.data !== 'string') {
            console.log("Received non-text message, ignoring");
            return;
        }

        const message = JSON.parse(event.data);
        console.log("Received message:", message);
        
        switch (message.message_type) {
            case "PeerList":
                console.log("Received PeerList message:", message);
                handlePeerListMessage(message);
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
            case "CallRequest":
                console.log("Received CallRequest message:", message);
                if (message.to_peers.includes(document.getElementById('peerId').value)) {
                    try {
                        // First check/request permissions
                        await navigator.mediaDevices.getUserMedia({ video: true, audio: true });
                        
                        setRemotePeerId(message.from_peer);
                        updateStatus(`Incoming call from ${remotePeerId}`);
                        
                        // Only send acceptance after permissions granted
                        sendSignal('CallResponse', {
                            room_id: document.getElementById('roomId').value,
                            from_peer: document.getElementById('peerId').value,
                            to_peer: remotePeerId,
                            accepted: true
                        });
                        await setupPeerConnection();
                    } catch (err) {
                        console.error('Permission denied or error accessing media devices:', err);
                        // Inform the caller that we can't accept
                        sendSignal('CallResponse', {
                            room_id: document.getElementById('roomId').value,
                            from_peer: document.getElementById('peerId').value,
                            to_peer: message.from_peer,
                            accepted: false,
                            reason: 'Failed to access media devices'
                        });
                        updateStatus('Failed to access camera/microphone. Please grant permissions and try again.', true);
                    }
                }
                break;
            case "CallResponse":
                await handleCallResponseMessage(message);
                break;
        }
    } catch (err) {
        console.error('Failed to process WebSocket message:', err);
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

export function sendSignal(messageType, data = {}) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        const message = {
            message_type: messageType,
            ...data
        };
        
        Object.keys(message).forEach(key => 
            message[key] === undefined && delete message[key]
        );
        
        console.log("Sending message:", message);
        ws.send(JSON.stringify(message));
    } else {
        console.error("WebSocket is not open");
    }
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
        try {
            await setupPeerConnection();
            // Rest of the handling...
        } catch (err) {
            console.error('Error setting up call:', err);
        }
    } else {
        updateStatus(`Call rejected by ${message.from_peer}: ${message.reason || 'No reason given'}`, true);
    }
}

async function handleOfferMessage(message) {
    console.log("Received offer from:", message.from_peer);
    setRemotePeerId(message.from_peer);
    
    // Get connection type from UI
    const connectionTypeSelect = document.getElementById('connectionType');
    console.log('Using connection type:', connectionTypeSelect);
    
    const constraints = {
        audio: {
            echoCancellation: true,
            noiseSuppression: true,
            autoGainControl: true
        },
        video: enableVideo
    };

    try {
        // 1. Setup peer connection first if needed
        if (!peerConnection) {
            console.log("Using connection type:", connectionType);
            await setupPeerConnection();
            // Note: ICE candidate handling is now managed in webrtc.js
        }

        // 2. Set the remote description (offer) before media handling
        console.log("Setting remote description (offer)");
        await peerConnection.setRemoteDescription(new RTCSessionDescription({
            type: 'offer',
            sdp: message.sdp
        }));
        isRemoteDescriptionSet = true;

        // 3. Get and add local media stream
        try {
            console.log('Requesting media with constraints:', constraints);
            const stream = await navigator.mediaDevices.getUserMedia(constraints);
            setLocalStream(stream);
            
            // Add local stream tracks to the connection
            stream.getTracks().forEach(track => {
                console.log('Adding track to peer connection:', track.kind);
                peerConnection.addTrack(track, stream);
            });
        } catch (err) {
            console.error('Error getting local media:', err);
            updateStatus('Failed to get local media: ' + err.message, true);
            return;
        }
        
        // 4. Process any buffered candidates
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

        // 5. Create answer
        console.log("Creating answer");
        const answer = await peerConnection.createAnswer();
        
        // 6. Set local description (answer)
        console.log("Setting local description (answer)");
        await peerConnection.setLocalDescription(answer);

        // 7. Wait for ICE gathering to complete
        if (peerConnection.iceGatheringState !== 'complete') {
            console.log('Waiting for ICE gathering to complete...');
            await new Promise(resolve => {
                const checkState = () => {
                    if (peerConnection.iceGatheringState === 'complete') {
                        peerConnection.removeEventListener('icegatheringstatechange', checkState);
                        resolve();
                    }
                };
                peerConnection.addEventListener('icegatheringstatechange', checkState);
            });
        }

        console.log('ICE gathering complete, sending answer');
        // 8. Send answer back to peer
        sendSignal('Answer', {
            room_id: document.getElementById('roomId').value,
            sdp: answer.sdp,
            from_peer: document.getElementById('peerId').value,
            to_peer: message.from_peer
        });

    } catch (err) {
        console.error('Error handling offer:', err);
        updateStatus('Failed to handle offer: ' + err.message, true);
    }
}

async function handleAnswerMessage(message) {
    console.log("Received answer from:", message.from_peer);
    try {
        setRemotePeerId(message.from_peer);
        if (!peerConnection) {
            console.log("No peer connection available for answer");
            return;
        }

        if (peerConnection.signalingState === "have-local-offer") {
            // Create and set the remote description
            const answerDesc = new RTCSessionDescription({
                type: 'answer',
                sdp: message.sdp
            });

            // Wait for ICE gathering to complete before setting remote description
            await new Promise((resolve) => {
                if (peerConnection.iceGatheringState === 'complete') {
                    resolve();
                } else {
                    peerConnection.onicegatheringstatechange = () => {
                        if (peerConnection.iceGatheringState === 'complete') {
                            resolve();
                        }
                    };
                }
            });

            console.log("Setting remote description (answer):", answerDesc);
            await peerConnection.setRemoteDescription(answerDesc);
            
            console.log("Remote description set successfully. Connection state:", {
                iceConnectionState: peerConnection.iceConnectionState,
                connectionState: peerConnection.connectionState,
                signalingState: peerConnection.signalingState,
                iceGatheringState: peerConnection.iceGatheringState
            });
        } else {
            console.log("Ignoring answer - not in have-local-offer state, current state:", peerConnection.signalingState);
        }

        if (iceCandidateQueue.length > 0) {
            console.log(`Processing ${iceCandidateQueue.length} queued ICE candidates`);
            while (iceCandidateQueue.length) {
                const {candidate, from_peer} = iceCandidateQueue.shift();
                try {
                    console.log('Processing queued ICE candidate:', candidate);
                    await peerConnection.addIceCandidate(candidate);
                } catch (err) {
                    console.error('Error adding queued ICE candidate:', err);
                }
            }
        }

        isNegotiating = false;
        if (iceCandidateBuffer.length > 0) {
            console.log(`Processing ${iceCandidateBuffer.length} buffered ICE candidates`);
            for (const candidate of iceCandidateBuffer) {
                try {
                    await peerConnection.addIceCandidate(candidate);
                } catch (err) {
                    console.error('Error adding buffered ICE candidate:', err);
                }
            }
            iceCandidateBuffer = [];
        }
    } catch (err) {
        console.error("Error handling answer:", err);
        console.error("Error details:", err.message);
        if (err.stack) {
            console.error("Stack trace:", err.stack);
        }
    }
}

async function handleIceCandidateMessage(message) {
    if (!peerConnection) {
        console.warn('No peer connection available for ICE candidate');
        return;
    }

    try {
        const candidateInit = JSON.parse(message.candidate);
        if (!peerConnection.remoteDescription) {
            // Buffer the candidate if remote description isn't set yet
            iceCandidateBuffer.push(candidateInit);
            console.log('Buffering ICE candidate until remote description is set');
            return;
        }

        await peerConnection.addIceCandidate(candidateInit);
        console.log('Successfully added ICE candidate');
    } catch (err) {
        console.error('Error adding ICE candidate:', err);
    }
}