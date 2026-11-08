FROM ontop/ontop-endpoint@sha256:e4baf868418947dbbe251e796aa1658890dee7973a1d8139fe5132cb6f457cf9
# Files are baked in (not bind-mounted) so the stack also works on Docker Desktop without a WSL distro socket.
COPY mapping.ttl /opt/ontop/input/mapping.ttl
COPY ontop.properties /opt/ontop/input/ontop.properties
COPY postgresql.jar /opt/ontop/jdbc/postgresql.jar
ENV ONTOP_MAPPING_FILE=/opt/ontop/input/mapping.ttl \
    ONTOP_PROPERTIES_FILE=/opt/ontop/input/ontop.properties \
    ONTOP_CORS_ALLOWED_ORIGINS=* \
    ONTOP_DEV_MODE=false
