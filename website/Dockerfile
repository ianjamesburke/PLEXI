# syntax=docker/dockerfile:1.7

# ---- Build stage ----------------------------------------------------------
FROM node:22-alpine AS build

# better-sqlite3 is a native module — needs a toolchain to compile.
RUN apk add --no-cache python3 make g++

WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci

COPY . .
RUN npm run build

# Drop dev deps for a lean runtime copy. Native binaries are already compiled.
RUN npm prune --omit=dev

# ---- Runtime stage --------------------------------------------------------
FROM node:22-alpine AS runtime

WORKDIR /app
ENV NODE_ENV=production
ENV HOST=0.0.0.0
ENV PORT=4321
ENV DB_PATH=/data/subscribers.db

COPY --from=build /app/dist ./dist
COPY --from=build /app/node_modules ./node_modules
COPY --from=build /app/package.json ./package.json

EXPOSE 4321

CMD ["node", "./dist/server/entry.mjs"]
