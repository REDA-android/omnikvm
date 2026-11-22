package main

import (
	"encoding/json"
	"testing"
)

func TestBuildBeaconJSON(t *testing.T) {
	raw, err := BuildBeaconJSON("peer-1", "test-host", 4433)
	if err != nil {
		t.Fatalf("BuildBeaconJSON failed: %v", err)
	}
	var b PeerBeacon
	if err := json.Unmarshal(raw, &b); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}
	if b.PeerID != "peer-1" || b.Hostname != "test-host" || b.QuicPort != 4433 {
		t.Fatalf("unexpected beacon: %+v", b)
	}
}
