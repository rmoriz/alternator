# GitHub Actions Setup

## Enabling Automated Pull Request Creation

To allow the dependency update workflow to automatically create pull requests, you need to enable the following repository setting:

### Repository Settings Configuration

1. Go to your repository on GitHub
2. Navigate to **Settings** > **Actions** > **General**
3. Scroll down to **Workflow permissions**
4. Check the box for **"Allow GitHub Actions to create and approve pull requests"**
5. Click **Save**

### Alternative: Using a Personal Access Token

If you prefer not to enable the above setting, you can create a Personal Access Token (PAT):

1. Create a PAT with `repo` scope in your GitHub settings
2. Add it as a repository secret named `PAT_TOKEN`
3. Update the workflow to use `token: ${{ secrets.PAT_TOKEN }}` instead of `${{ secrets.GITHUB_TOKEN }}`

## Current Workflow Behavior

The dependency update workflow (`.github/workflows/security.yml`) currently:

- ✅ **Works**: Updates dependencies, runs tests, performs security audits
- ⚠️ **Fallback**: If PR creation fails, it pushes a branch with instructions for manual PR creation
- 🔧 **Requires**: Repository setting change OR PAT token for full automation

## Security Considerations

- The recommended approach is enabling the repository setting as it's more secure
- PAT tokens should be used sparingly and rotated regularly
- The fallback branch creation ensures dependency updates are never lost