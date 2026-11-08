# Linux/WSL development deployment: `just backend` builds this binary first.
FROM ubuntu:22.04
RUN apt-get update && apt-get install -y --no-install-recommends libstdc++6 ca-certificates curl socat tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home rdf \
    && mkdir /data && chown rdf:rdf /data
COPY target/debug/rdftool /usr/local/bin/rdftool
COPY deploy/local/backend-entrypoint.sh /usr/local/bin/backend-entrypoint
COPY plugins/inference/example-rules-config.json /config/inference.json
COPY plugins/semantic-search/example-config.json /config/semantic.json
COPY deploy/local/tenants /config/tenants
USER rdf
ENTRYPOINT ["/usr/bin/tini", "-g", "--", "bash", "/usr/local/bin/backend-entrypoint"]
