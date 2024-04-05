# Getting-Started

## Prerequisite

Ubuntu:

```
sudo apt update && sudo apt upgrade -y
sudo apt install -y protobuf-compiler libprotobuf-dev
```

Alpine Linux:

```
sudo apk add protoc protobuf-dev
```

macOS:

```
brew install protobuf
```

## Start

```
cargo run --bin tonic-template-server
```

## Extra Tools

### grpcurl

Ubuntu:

```
# install
curl -sSL "https://github.com/fullstorydev/grpcurl/releases/download/v1.8.7/grpcurl_1.8.7_linux_x86_64.tar.gz" | sudo tar -xz -C /usr/local/bin

# usage example
grpcurl -plaintext -import-path ./protos/helloworld -proto helloworld.proto -d '{"name": "Tonic"}' '[::1]:50051' helloworld.Greeter/S
ayHello
```
