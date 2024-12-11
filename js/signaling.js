let ws;
let isDisconnecting = false;
let activeCallRequests = new Set();
let reconnectAttempts = 0;
const MAX_RECONNECT_ATTEMPTS = 5;

async function connect() {
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
    sendSignal('Join', {
        room_id: roomId,
        peer_id: peerId
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
                const currentPeerId = document.getElementById('peerId').value;
                const otherPeers = message.peers.filter(p => p !== currentPeerId);
                
                // Check if any peers we were connected to are no longer in the list
                if (peerConnection && remotePeerId && !otherPeers.includes(remotePeerId)) {
                    console.log(`Peer ${remotePeerId} disconnected`);
                    cleanupConnection();
                    updateStatus('Peer disconnected');
                }
                
                // Update the peer list UI
                const peerListDiv = document.getElementById('selectablePeerList');
                peerListDiv.innerHTML = otherPeers.map(peerId => `
                    <div class="peer-item">
                        <input type="checkbox" id="peer_${peerId}" value="${peerId}">
                        <label for="peer_${peerId}">${peerId}</label>
                    </div>
                `).join('');
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
                        
                        remotePeerId = message.from_peer;
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
    
    if (!isDisconnecting) {
        cleanupConnection();
    }
}

function sendSignal(messageType, data = {}) {
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

function disconnect() {
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
        peerConnection = null;
    }

    if (localStream) {
        localStream.getTracks().forEach(track => track.stop());
        localStream = null;
    }

    // Remove remote audio element if it exists
    const remoteAudio = document.getElementById('remoteAudio');
    if (remoteAudio) {
        remoteAudio.srcObject = null;
        remoteAudio.remove();
    }

    remotePeerId = null;
    document.getElementById('audioStatus').textContent = 'Audio status: Not in call';
}

async function handleCallResponseMessage(message) {
    // Ignore if we're already processing a call response
    if (activeCallRequests.has(message.from_peer)) {
        console.log(`Already processing call response from ${message.from_peer}`);
        return;
    }

    if (message.to_peer === document.getElementById('peerId').value && message.accepted) {
        try {
            activeCallRequests.add(message.from_peer);
            remotePeerId = message.from_peer;
            updateStatus(`Call accepted by ${remotePeerId}`);
            
            // Only proceed if we don't have an active connection
            if (!peerConnection || peerConnection.connectionState === 'closed') {
                await setupPeerConnection();
                
                // Create and send offer
                const offer = await peerConnection.createOffer({
                    offerToReceiveAudio: true
                });
                await peerConnection.setLocalDescription(offer);
                
                sendSignal('Offer', {
                    room_id: document.getElementById('roomId').value,
                    sdp: offer.sdp,
                    from_peer: document.getElementById('peerId').value,
                    to_peer: remotePeerId
                });
            }
        } finally {
            activeCallRequests.delete(message.from_peer);
        }
    }
}

async function handleOfferMessage(message) {
    console.log("Received offer from:", message.from_peer);
    try {
        if (!peerConnection) {
            await setupPeerConnection();
        }

        if (peerConnection.signalingState !== "stable") {
            console.log("Ignoring offer - not in stable state");
            return;
        }

        const offerDesc = {
            type: 'offer',
            sdp: message.sdp
        };

        console.log("Setting remote description (offer)");
        await peerConnection.setRemoteDescription(new RTCSessionDescription(offerDesc));

        console.log("Creating answer");
        const answer = await peerConnection.createAnswer();
        
        console.log("Setting local description (answer)");
        await peerConnection.setLocalDescription(answer);

        console.log("Sending answer to:", message.from_peer);
        sendSignal('Answer', {
            room_id: document.getElementById('roomId').value,
            sdp: answer.sdp,
            from_peer: document.getElementById('peerId').value,
            to_peer: message.from_peer
        });
    } catch (err) {
        console.error("Error handling offer:", err);
    }
}

async function handleAnswerMessage(message) {
    console.log("Received answer from:", message.from_peer);
    try {
        if (!peerConnection) {
            console.log("No peer connection available for answer");
            return;
        }

        if (peerConnection.signalingState === "have-local-offer") {
            console.log("Setting remote description (answer)");
            await peerConnection.setRemoteDescription(new RTCSessionDescription({
                type: 'answer',
                sdp: message.sdp
            }));
        } else {
            console.log("Ignoring answer - not in have-local-offer state, current state:", peerConnection.signalingState);
        }
    } catch (err) {
        console.error("Error handling answer:", err);
    }
}

async function handleIceCandidateMessage(message) {
    if (!peerConnection) {
        console.warn('Received ICE candidate but no peer connection exists');
        return;
    }
    
    try {
        const candidate = new RTCIceCandidate({
            candidate: message.candidate.candidate,
            sdpMid: message.candidate.sdpMid,
            sdpMLineIndex: message.candidate.sdpMLineIndex
        });
        
        console.log('Adding ICE candidate:', candidate);
        await peerConnection.addIceCandidate(candidate);
    } catch (err) {
        console.error('Error adding ICE candidate:', err);
    }
}

async function setupPeerConnection() {
    try {
        // Get STUN server configuration from input fields
        const stunServer = document.getElementById('stunServer').value || '127.0.0.1';
        const stunPort = document.getElementById('stunPort').value || '3478';
        const stunUrl = `stun:${stunServer}:${stunPort}`;

        const configuration = {
            iceServers: [
                { urls: stunUrl },
                { urls: 'stun:stun.l.google.com:19302' }  // Fallback public STUN server
            ],
            iceTransportPolicy: 'all',
            bundlePolicy: 'max-bundle',
            rtcpMuxPolicy: 'require',
            iceCandidatePoolSize: 10
        };

        // Only create a new connection if one doesn't exist or is closed
        if (peerConnection && peerConnection.connectionState !== 'closed') {
            console.log("Peer connection already exists");
            return peerConnection;
        }

        // Use existing stream if available
        if (!localStream) {
            console.log("Getting user media");
            try {
                const constraints = {
                    audio: {
                        echoCancellation: true,
                        noiseSuppression: true,
                        autoGainControl: true
                    },
                    video: enableVideo
                };
                
                localStream = await navigator.mediaDevices.getUserMedia(constraints);
                updateVideoUI();
            } catch (err) {
                console.error("Failed to get media:", err);
                throw new Error("Could not access media devices. Please check permissions.");
            }
        }

        console.log("Creating new RTCPeerConnection with configuration:", configuration);
        peerConnection = new RTCPeerConnection(configuration);

        // Add connection monitoring
        peerConnection.onconnectionstatechange = handleConnectionStateChange;
        peerConnection.oniceconnectionstatechange = handleIceConnectionStateChange;
        peerConnection.ontrack = handleTrackEvent;
        peerConnection.onicecandidate = handleIceCandidate;

        // Add local stream tracks to the connection
        localStream.getTracks().forEach(track => {
            console.log(`Adding track to peer connection: ${track.kind}`);
            peerConnection.addTrack(track, localStream);
        });

        return peerConnection;
    } catch (err) {
        console.error('Error setting up peer connection:', err);
        updateStatus('Failed to access microphone. Please grant permissions and try again.', true);
        throw err;
    }
}

// Call this before connecting
// window.addEventListener('load', checkAudioPermissions); 