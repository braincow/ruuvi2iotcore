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

release-cross-build-armv7:
	cross build --release --target armv7-unknown-linux-gnueabihf

clean:
	rm -rf log
	rm -rf target
	# if rpm's were built, get rid of the files there
	rm .rpm/CHANGELOG.md .rpm/LICENSE .rpm/example_gateway_config.json .rpm/log4rs.yaml .rpm/ruuvi2iotcore.yaml || true

copy-rpm-extra-files:
	cp CHANGELOG.md .rpm
	cp LICENSE .rpm
	cp example_gateway_config.json .rpm
	cp log4rs.yaml .rpm
	cp ruuvi2iotcore.yaml .rpm

release-rpm-armv7: release-cross-build-armv7 copy-rpm-extra-files
	cargo rpm build --target armv7-unknown-linux-gnueabihf --no-cargo-build

release-rpm: release-build copy-rpm-extra-files
	cargo rpm build --no-cargo-build
