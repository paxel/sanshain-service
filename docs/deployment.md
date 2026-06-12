# Deployment Guide

This guide covers production-ready deployment of Sanshain Service on Kubernetes.

## Kubernetes (via Kustomize)

The `deploy/kubernetes` directory contains base manifests suitable for customization via Kustomize.

### Prerequisites

- A Kubernetes cluster.
- `kubectl` installed and configured.

### Configuration

1. **Namespace**: It's recommended to create a dedicated namespace.
   ```bash
   kubectl create namespace sanshain
   ```

2. **Secrets**: Update `deploy/kubernetes/secret.yaml` with your base64-encoded values for `DATABASE_URL` and `INITIAL_ADMIN_PASSWORD`.
   ```bash
   echo -n "sqlite:///data/sanshain.db" | base64
   echo -n "your-secure-password" | base64
   ```

3. **Ingress**: Update `deploy/kubernetes/ingress.yaml` with your actual hostnames and TLS secret name.

### Deployment

To apply the manifests using Kustomize:

```bash
kubectl apply -k deploy/kubernetes -n sanshain
```

This will create the following resources:
- `ConfigMap`: Application settings.
- `Secret`: Sensitive credentials.
- `PersistentVolumeClaim`: 1Gi storage for SQLite (mounted at `/data`).
- `Deployment`: 2 replicas of the Sanshain Service.
- `Service`: ClusterIP service for internal access.
- `Ingress`: External access with TLS support.

---

## Helm Chart

For more complex deployments or automated distribution, a Helm chart is provided in `deploy/helm/sanshain`.

### Prerequisites

- [Helm](https://helm.sh/docs/intro/install/) v3+.

### Installation

1. **Install with default settings (SQLite)**:
   ```bash
   helm install sanshain deploy/helm/sanshain -n sanshain --create-namespace
   ```

2. **Install with external PostgreSQL**:
   Create a `my-values.yaml` file:
   ```yaml
   persistence:
     enabled: false # No local PVC needed for SQLite
   
   secrets:
     databaseUrl: "postgres://user:password@postgres-host:5432/sanshain"
   ```
   Then install:
   ```bash
   helm install sanshain deploy/helm/sanshain -f my-values.yaml -n sanshain
   ```

### Common Configuration Options

See `deploy/helm/sanshain/values.yaml` for a complete list of parameters. Key options include:

| Parameter | Description | Default |
|-----------|-------------|---------|
| `replicaCount` | Number of pods to run | `1` |
| `image.tag` | Image tag to deploy | (Chart `appVersion`) |
| `ingress.enabled` | Enable Ingress resource | `false` |
| `persistence.enabled` | Enable persistent storage for SQLite | `true` |
| `persistence.size` | Size of the persistent volume | `1Gi` |
| `config.initialAdminUsername` | Initial admin username | `root` |
| `secrets.initialAdminPassword` | Initial admin password (if empty, random password is logged) | `""` |

### Uninstallation

```bash
helm uninstall sanshain -n sanshain
```

## Persistence Notes

### SQLite
When using SQLite (default), the database file is stored at `/data/sanshain.db`. The `persistence.enabled` setting must be `true` to ensure data survives pod restarts.

### PostgreSQL
If using PostgreSQL, set `persistence.enabled` to `false` (unless you need persistent storage for other purposes) and provide the `DATABASE_URL` in `secrets.databaseUrl`.

## Security

- **Non-Root**: The container is configured to run as a non-root user (UID 1000).
- **Security Context**: Standard Pod and Container security contexts are applied in both Kustomize and Helm templates.
- **TLS**: Ingress templates support TLS termination.
