# Build Stage 
FROM node:20 AS build

WORKDIR /usr/src/app

COPY package*.json .

RUN npm install

COPY . .

ARG DATABASE_URL=file:./db/dev.db
ARG DOMAIN=yourdomain.tld

ENV DATABASE_URL=${DATABASE_URL}
ENV DOMAIN=${DOMAIN}

RUN mv ./src/config/traefikConfigTemplate.ts.example ./src/config/traefikConfigTemplate.ts && \
npm run prisma:init && \
npm run build

# Production Stage 
FROM node:20 AS production 

ARG NODE_ENV=production
ARG DATABASE_URL=file:./db/dev.db
ARG DOMAIN=yourdomain.tld
ARG AUTHELIAADDRESS=http://192.168.1.1:9091

ENV NODE_ENV=${NODE_ENV}
ENV DATABASE_URL=${DATABASE_URL}
ENV DOMAIN=${DOMAIN}
ENV AUTHELIAADDRESS=${AUTHELIAADDRESS}

WORKDIR /usr/src/app

COPY --from=build package*.json .
COPY --from=build /usr/src/app/prisma/schema.prisma ./schema.prisma

RUN npm ci --omit=dev && \
npm run prisma:init

COPY --from=build /usr/src/app/dist ./dist

CMD [ "node", "dist/index.js" ]