package main

import (
	"encoding/json"
	"fmt"
	"net"
	"os"
	"runtime"
	"time"
)

type PeerBeacon struct {
	PeerID         string `json:"peer_id"`
	Hostname       string `json:"hostname"`
	OS             string `json:"os"`
	QuicPort       uint16 `json:"quic_port"`
	WebRtcPort     uint16 `json:"webrtc_port"`
	FingerprintHex string `json:"fingerprint_hex"`
	Timestamp      int64  `json:"timestamp_ms"`
}

func BuildBeaconJSON(peerID, hostname string, quicPort uint16) ([]byte, error) {
	b := PeerBeacon{
		PeerID:         peerID,
		Hostname:       hostname,
		OS:             runtime.GOOS,
		QuicPort:       quicPort,
		WebRtcPort:     quicPort + 1,
		FingerprintHex: "go-mdns-beacon",
		Timestamp:      time.Now().UnixMilli(),
	}
	return json.Marshal(b)
}

func main() {
	host, _ := os.Hostname()
	payload, err := BuildBeaconJSON("go-mdns-helper", host, 4433)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
	conn, err := net.DialUDP("udp4", nil, &net.UDPAddr{
		IP:   net.IPv4(127, 0, 0, 1),
		Port: 53539,
	})
	if err != nil {
		fmt.Println(string(payload))
		return
	}
	defer conn.Close()
	_, _ = conn.Write(payload)
	fmt.Println(string(payload))
}
