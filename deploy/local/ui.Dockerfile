FROM node:22.23.4-bookworm-slim AS build
ENV ASTRO_TELEMETRY_DISABLED=1
WORKDIR /app
COPY apps/studio/package*.json ./
RUN npm ci
COPY apps/studio/ ./
RUN npm run check && npm run build
FROM nginx:1.28-alpine
COPY deploy/local/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html
