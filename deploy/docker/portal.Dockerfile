# Bifrost review portal (#67). Build the Vite app against the HTTP backend, then
# serve the static bundle from nginx, proxying /api to the API. Context = repo root.
FROM node:22-slim AS build
WORKDIR /app
COPY portal/package.json portal/package-lock.json ./
RUN npm ci
COPY portal/ ./
# Use the real backend (relative /api), not the in-browser mock.
ENV VITE_API=http
RUN npm run build

FROM nginx:1.27-alpine AS runtime
COPY deploy/docker/portal-nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html
EXPOSE 80
