# Hot-reload Studio: Astro dev server. `docker compose watch` syncs source edits in (see compose.dev.yml).
FROM node:22.23.4-bookworm-slim
ENV ASTRO_TELEMETRY_DISABLED=1
WORKDIR /app
COPY apps/studio/package*.json ./
RUN npm ci --no-audit --no-fund
COPY apps/studio/ ./
EXPOSE 4321
CMD ["npx", "astro", "dev", "--host", "0.0.0.0", "--port", "4321"]
