# Kubarr Screenshots

This directory contains screenshots and visual documentation for the Kubarr project. The screenshots are referenced in the main README.md and documentation files.

## Screenshot Requirements

To complete the documentation, the following screenshots are needed:

### 1. Authentication & Dashboard

#### `login.png`
- **Purpose:** Show the login page with authentication options
- **Content Required:**
  - Username/email and password input fields
  - "Sign In" button
  - Optional "Remember Me" checkbox
  - 2FA/TOTP input field (if enabled)
  - Kubarr logo and branding
- **Recommended Size:** 1280x800px
- **Notes:** Use a clean, professional appearance. No real credentials visible.

#### `dashboard.png`
- **Purpose:** Show the main dashboard overview
- **Content Required:**
  - Cluster resource summary (CPU, Memory, Pods)
  - List of deployed applications with status indicators
  - Recent activity or events panel
  - Navigation sidebar/menu
  - Quick action buttons
- **Recommended Size:** 1920x1080px
- **Notes:** Show a realistic cluster state with 3-5 deployed applications

### 2. Application Management

#### `catalog.png`
- **Purpose:** Display the application catalog/marketplace
- **Content Required:**
  - Grid or list view of available applications
  - Application icons, names, and descriptions
  - Categories or filters (e.g., Databases, Monitoring, Storage)
  - Search bar
  - "Deploy" or "Install" buttons
- **Recommended Size:** 1920x1080px
- **Notes:** Include popular apps like PostgreSQL, Redis, Prometheus, Grafana, etc.

#### `deployed-apps.png`
- **Purpose:** Show the view of currently deployed applications
- **Content Required:**
  - List of running applications with status (Running, Pending, Failed)
  - Resource usage per application (CPU, Memory)
  - Number of replicas/pods
  - Actions menu (Scale, Update, Delete, View Logs, etc.)
  - Namespace selector
- **Recommended Size:** 1920x1080px
- **Notes:** Show diverse statuses to demonstrate monitoring capabilities

#### `logs-viewer.png`
- **Purpose:** Demonstrate real-time log viewing capabilities
- **Content Required:**
  - Application/pod selector dropdown
  - Log output area with timestamps
  - Search/filter input
  - Auto-scroll toggle
  - Log level filters (INFO, WARN, ERROR)
  - Download logs button
- **Recommended Size:** 1920x1080px
- **Notes:** Use realistic log entries with proper formatting and syntax highlighting

### 3. Advanced Features

#### `file-browser.png`
- **Purpose:** Show ConfigMap and Secret management interface
- **Content Required:**
  - File tree or list showing ConfigMaps and Secrets
  - Editor pane with YAML/JSON content
  - Create, Edit, Delete action buttons
  - Namespace selector
  - Key-value editor for ConfigMaps
  - Toggle for showing/hiding Secret values
- **Recommended Size:** 1920x1080px
- **Notes:** Mask any sensitive values. Show YAML syntax highlighting.

#### `user-management.png`
- **Purpose:** Display user administration interface
- **Content Required:**
  - Table of users with columns: Username, Email, Role, Status, Last Login
  - Add/Invite User button
  - Edit and Delete actions per user
  - User status indicators (Active, Suspended, Pending)
  - Search/filter bar
  - Pagination controls
- **Recommended Size:** 1920x1080px
- **Notes:** Use placeholder names and emails (e.g., john.doe@example.com)

#### `role-management.png`
- **Purpose:** Show RBAC role configuration interface
- **Content Required:**
  - List of roles (Admin, Developer, Viewer, Custom roles)
  - Permissions matrix or detailed permissions list
  - Create Role and Edit Role buttons
  - Permission categories (Deployments, Services, ConfigMaps, etc.)
  - Checkboxes for Read, Write, Delete permissions
  - Role assignment count
- **Recommended Size:** 1920x1080px
- **Notes:** Clearly show the hierarchy of permissions

## Screenshot Guidelines

### General Requirements

1. **Consistency:**
   - Use the same theme/color scheme across all screenshots
   - Maintain consistent UI elements (header, sidebar, etc.)
   - Use the same font sizes and spacing

2. **Quality:**
   - Minimum resolution: 1280x800px
   - Recommended resolution: 1920x1080px
   - Use PNG format for crisp text rendering
   - Avoid JPEG artifacts

3. **Content:**
   - Use realistic but anonymized data
   - Avoid empty states unless specifically demonstrating them
   - Show meaningful metrics and values
   - Use proper terminology and labels

4. **Privacy & Security:**
   - No real API keys, tokens, or secrets
   - Use placeholder domains (example.com, example.org)
   - Anonymize usernames and emails
   - Mask sensitive configuration values

### Capturing Screenshots

Recommended tools:
- **Browser DevTools:** Set viewport to 1920x1080 for consistency
- **macOS:** Cmd+Shift+4 (select area) or Cmd+Shift+5 (screenshot utility)
- **Windows:** Snipping Tool or Win+Shift+S
- **Linux:** gnome-screenshot, flameshot, or shutter

### Naming Convention

- Use lowercase with hyphens: `feature-name.png`
- Be descriptive but concise
- Match the filenames referenced in README.md

## Current Status

| Screenshot | Status | Notes |
|------------|--------|-------|
| `login.png` | ⏳ Pending | Need to capture from deployed instance |
| `dashboard.png` | ⏳ Pending | Need realistic cluster data |
| `catalog.png` | ⏳ Pending | Design app catalog UI first |
| `deployed-apps.png` | ⏳ Pending | Need multiple deployed apps for demo |
| `logs-viewer.png` | ⏳ Pending | Implement log viewer UI |
| `file-browser.png` | ⏳ Pending | ConfigMap/Secret browser implementation |
| `user-management.png` | ⏳ Pending | User admin UI needed |
| `role-management.png` | ⏳ Pending | RBAC UI implementation |

## Contributing Screenshots

If you'd like to contribute screenshots:

1. Deploy Kubarr to a local or test cluster
2. Populate with realistic demo data (see `scripts/setup-demo-data.sh` if available)
3. Capture screenshots following the guidelines above
4. Optimize images (use tools like `optipng` or `pngquant`)
5. Submit via pull request with updated status table

## Questions?

If you have questions about screenshot requirements or need clarification on specific content, please open an issue or discussion on GitHub.
