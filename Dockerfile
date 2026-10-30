# Bedrock sync server.
#   docker build -t bedrock-server .
#   docker run -d -p 8787:8787 -v bedrock-data:/data bedrock-server
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p bedrock-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/bedrock-server /usr/local/bin/bedrock-server
ENV BEDROCK_BIND=0.0.0.0:8787 \
    BEDROCK_DATA=/data \
    BEDROCK_REGISTRATION=first-only
VOLUME /data
EXPOSE 8787
CMD ["bedrock-server"]
