FROM node:22-slim
WORKDIR /app
COPY package.json ./
RUN npm install --omit=dev --no-audit --no-fund
COPY server.mjs ./
COPY allow.json /config/federation.json
USER node
EXPOSE 8095
CMD ["node", "server.mjs"]
