create-test-cert:
	openssl req -x509 -sha256 -nodes -days 365 -newkey rsa:2048 -keyout test.key -out test.crt

debug-pcap-permissions:
	sudo setcap 'cap_net_raw,cap_net_admin+eip' target/debug/ruuvi2iotcore

debug-build:
	cargo build

debug-run: debug-build debug-pcap-permissions
	cargo run

