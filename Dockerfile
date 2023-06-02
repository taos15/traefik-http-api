# Use Node.js 20 as the base image
FROM node:20 AS builder

# Set the working directory inside the container
WORKDIR /app

COPY package.json /app/
COPY yarn.lock /app/

RUN yarn install 

COPY . .

RUN yarn run build2 && \
yarn run build 

RUN npx prisma generate


CMD [ "node", "/app/dist/src/index.js" ]

