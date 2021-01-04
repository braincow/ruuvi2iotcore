download-roots:
	curl -O https://pki.goog/roots.pem

create-test-cert:
	openssl req -x509 -sha256 -nodes -days 365 -newkey rsa:2048 -keyout test.key -out test.crt

debug-pcap-permissions: debug-build
	sudo setcap 'cap_net_raw,cap_net_admin+eip' target/debug/ruuvi2iotcore

debug-build:
	cargo build

debug-cross-build-armv7:
	cross build --target armv7-unknown-linux-gnueabihf

debug-run: debug-pcap-permissions
	rm -rf log
	RUST_BACKTRACE=1 cargo run -- -w . -c ruuvi2iotcore.yaml -l log4rs.yaml

release-build:
	cargo build --release

cross-build-armv7:
	cross build --release --target armv7-unknown-linux-gnueabihf

clean:
	rm -rf log
	rm -rf target

rpm-armv7: cross-build-armv7
	cargo rpm build --target armv7-unknown-linux-gnueabihf --no-cargo-build

rpm: release-build
	cargo rpm build --no-cargo-build
