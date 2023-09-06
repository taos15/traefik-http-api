# Build Stage 
FROM node:20-slim AS base
ARG DATABASE_URL=file:./db/dev.db
ARG DOMAIN=yourdomain.tld
ARG AUTHELIAADDRESS=http://192.168.1.1:9091

ENV DATABASE_URL=${DATABASE_URL}
ENV DOMAIN=${DOMAIN}
ENV AUTHELIAADDRESS=${AUTHELIAADDRESS}
ENV PNPM_HOME="/pnpm"
ENV PATH="$PNPM_HOME:$PATH"
RUN corepack enable
COPY . /usr/src/app
WORKDIR /usr/src/app
RUN apt-get update -y && apt-get install -y openssl

RUN mv src/config/traefikConfigTemplate.ts.example src/config/traefikConfigTemplate.ts


FROM base AS prod-deps
RUN --mount=type=cache,id=pnpm,target=/pnpm/store pnpm install --prod --frozen-lockfile

FROM base AS build
RUN --mount=type=cache,id=pnpm,target=/pnpm/store pnpm install --frozen-lockfile
RUN pnpm run prisma:generate && \
pnpm run build

# Production Stage
FROM base
COPY --from=prod-deps /usr/src/app/node_modules /usr/src/app/node_modules
COPY --from=build /usr/src/app/dist /usr/src/app/dist

RUN pnpm run prisma:init

EXPOSE 4000
CMD [ "node", "dist/index.js" ]