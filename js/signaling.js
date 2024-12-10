let ws;
let isDisconnecting = false;
let activeCallRequests = new Set();

async function connect() {
    try {
        const peerId = document.getElementById('peerId').value || 
                      `user-${Math.random().toString(36).substr(2, 5)}`;
        document.getElementById('peerId').value = peerId;
        
        ws = new WebSocket('ws://127.0.0.1:8080');
        console.log('WebSocket connection attempting...');
        
        ws.onopen = handleWebSocketOpen;
        ws.onmessage = handleWebSocketMessage;
        ws.onerror = handleWebSocketError;
        ws.onclose = handleWebSocketClose;

        const constraints = {
            audio: {
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true
            },
            video: false
        };

        try {
            localStream = await navigator.mediaDevices.getUserMedia(constraints);
            document.getElementById('audioStatus').textContent = 'Audio status: Ready';
        } catch (err) {
            console.error('Error getting local stream:', err);
            updateStatus('Error getting local stream: ' + err.message, true);
            return null;
        }
    } catch (err) {
        updateStatus('Error: ' + err.message, true);
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
                    remotePeerId = message.from_peer;
                    updateStatus(`Incoming call from ${remotePeerId}`);
                    // Accept call and create peer connection
                    sendSignal('CallResponse', {
                        room_id: document.getElementById('roomId').value,
                        from_peer: document.getElementById('peerId').value,
                        to_peer: remotePeerId,
                        accepted: true
                    });
                    await setupPeerConnection();
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
    if (message.to_peer === document.getElementById('peerId').value && message.accepted) {
        remotePeerId = message.from_peer;
        updateStatus(`Call accepted by ${remotePeerId}`);
        
        // Create RTCPeerConnection and continue with call setup
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

async function checkAudioPermissions() {
    try {
        const stream = await navigator.mediaDevices.getUserMedia({ 
            audio: {
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true
            }
        });
        
        console.log('Audio permissions granted:', {
            tracks: stream.getTracks().map(t => ({
                kind: t.kind,
                enabled: t.enabled,
                muted: t.muted,
                readyState: t.readyState
            }))
        });
        
        // Stop the test stream
        stream.getTracks().forEach(track => track.stop());
        return true;
    } catch (err) {
        console.error('Audio permission check failed:', err);
        updateStatus('Microphone access denied. Please grant permission.', true);
        return false;
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

// Call this before connecting
window.addEventListener('load', checkAudioPermissions); 