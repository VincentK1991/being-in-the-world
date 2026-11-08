FROM maven:3.9.11-eclipse-temurin-21 AS build
WORKDIR /build
COPY pom.xml .
COPY src ./src
RUN mvn -B -q package dependency:copy-dependencies -DoutputDirectory=target/lib
FROM eclipse-temurin:21-jre-jammy
RUN apt-get update && apt-get install -y --no-install-recommends python3 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /build/target/classes ./classes
COPY --from=build /build/target/lib ./lib
COPY server.py .
EXPOSE 8083
CMD ["python3","server.py"]
