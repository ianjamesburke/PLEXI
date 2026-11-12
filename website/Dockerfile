# syntax=docker/dockerfile:1.7

# ---- Build stage ----------------------------------------------------------
FROM node:22-alpine AS build

WORKDIR /app

COPY website/package.json website/package-lock.json ./
RUN npm ci

COPY website/ .

# The install routes import these bootstraps from outside website/.
COPY install.sh /install.sh
COPY scripts/install-windows.ps1 /scripts/install-windows.ps1

RUN npm run build

# Drop dev deps for a lean runtime copy.
RUN npm prune --omit=dev

# ---- Runtime stage --------------------------------------------------------
FROM node:22-alpine AS runtime

WORKDIR /app
ENV NODE_ENV=production
ENV HOST=0.0.0.0
ENV PORT=4321

COPY --from=build /app/dist ./dist
COPY --from=build /app/node_modules ./node_modules
COPY --from=build /app/package.json ./package.json

EXPOSE 4321

CMD ["node", "./dist/server/entry.mjs"]
