# Use Node.js 20 as the base image
FROM node:20

# Set the working directory inside the container
WORKDIR /app

# Copy package.json, package-lock.json, and pnpm-lock.yaml to the working directory
COPY package*.json pnpm-lock.yaml ./

# Install pnpm globally
RUN npm install -g pnpm

# Install dependencies using pnpm
RUN pnpm install --frozen-lockfile

# Copy the rest of the application code to the working directory
COPY . .

# Build the TypeScript project
RUN pnpm run build

# Expose the port your Express app is listening on (replace 3000 with your app's port if needed)
EXPOSE 4000

# Start the Express app
CMD ["pnpm", "start"]
