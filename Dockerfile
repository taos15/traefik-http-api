# Build Stage 
FROM node:20-slim AS ui-build
ARG VITE_SERVER_API=${SERVER_API:-http://localhost:4000}

ENV VITE_SERVER_API=${VITE_SERVER_API}
ENV PNPM_HOME="/pnpm"
ENV PATH="$PNPM_HOME:$PATH"
RUN corepack enable
RUN apt-get update -y && apt-get install -y openssl git \
&& apt-get clean

RUN git clone https://github.com/taos15/traefik-http-webui
WORKDIR /traefik-http-webui
RUN --mount=type=cache,id=pnpm,target=/pnpm/store pnpm install --frozen-lockfile
RUN pnpm run build && \
mv dist UI

# Build Stage 
FROM node:20-slim AS base
ARG DATABASE_URL=file:./db/dev.db
ARG DOMAIN=yourdomain.tld
ARG AUTHELIAADDRESS=http://192.168.1.1:9091

ENV DATABASE_URL=${DATABASE_URL}
ENV DOMAIN=${DOMAIN}
ENV AUTHELIAADDRESS=${AUTHELIAADDRESS}
ENV VITE_SERVER_API=${VITE_SERVER_API}
ENV PNPM_HOME="/pnpm"
ENV PATH="$PNPM_HOME:$PATH"
RUN corepack enable
COPY . /app
WORKDIR /app
RUN apt-get update -y && apt-get install -y openssl \
&& apt-get clean

RUN mv src/config/traefikConfigTemplate.ts.example src/config/traefikConfigTemplate.ts


FROM base AS prod-deps
RUN --mount=type=cache,id=pnpm,target=/pnpm/store pnpm install --prod --frozen-lockfile

FROM base AS build
RUN --mount=type=cache,id=pnpm,target=/pnpm/store pnpm install --frozen-lockfile
RUN pnpm run prisma:generate && \
pnpm run build

# Production Stage
FROM base
COPY --from=prod-deps /app/node_modules /app/node_modules
COPY --from=build /app/dist /app/dist
COPY --from=ui-build /traefik-http-webui/UI /app/UI

RUN pnpm run prisma:init

EXPOSE 4000
VOLUME /app
CMD [ "node", "dist/index.js" ]