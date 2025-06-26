# Testomat Integration

This directory contains the configuration and scripts for sending test reports to Testomat.io.

## Files

- `testomat.yml` - Testomat configuration file
- `.envrc.testomat.sample` - Sample environment variables file
- `send-to-testomat.sh` - Script to send reports to Testomat (curl-based)
- `package.json` - npm configuration for Testomat CLI

## Setup

### 1. Get Testomat Credentials

1. Sign up for a Testomat.io account at https://app.testomat.io
2. Go to Union bridge project
3. Get your API key and project ID from the Testomat dashboard

### 2. Configure Environment Variables

Copy the sample environment file and fill in your credentials:

```bash
cp .envrc.testomat.sample .envrc.testomat
```

Edit `.envrc.testomat` and replace the placeholder values:

```bash
export TESTOMAT_API_KEY="your_actual_api_key_here"
export TESTOMAT_PROJECT_ID="your_actual_project_id_here"
```

### 3. Load Environment Variables

Source the environment file:

```bash
source .envrc.testomat
```

Or add it to your shell profile to load automatically.

## Usage

### Option 1: Using Testomat CLI (Recommended)

The Testomat CLI provides the most reliable way to send reports. It's already installed in this directory.

#### Prerequisites
Make sure you have Node.js installed (via nvm or other method).

#### Send a Test Report

```bash
# Navigate to the testomat directory
cd qa-tools/testomat

# Set up nvm and Node.js (if using nvm)
export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"
nvm use v22.13.0  # or your preferred version

# Set the correct environment variables (note: CLI uses TESTOMATIO_ prefix)
export TESTOMATIO_API_KEY="your_actual_api_key_here"
export TESTOMATIO_PROJECT_ID="your_actual_project_id_here"

# Send the report using the Testomat CLI
npx @testomatio/reporter xml ../reports/tx_dispatcher.xml
```

#### Example credentials:
```bash
export NVM_DIR="$HOME/.nvm" && [ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh" && nvm use v22.13.0 && export TESTOMATIO_API_KEY=<API KEY> && export TESTOMATIO_PROJECT_ID=union-bridge && npx @testomatio/reporter xml ../reports/tx_dispatcher.xml
```

### Option 2: Using Custom Script (Alternative)

After running your tests and generating a report, send it to Testomat:

```bash
# Navigate to the testomat directory
cd qa-tools/testomat

# Send the default report (../reports/tx_dispatcher.xml)
./send-to-testomat.sh

# Send a specific report file
./send-to-testomat.sh ../reports/your_report.xml
```

**Note**: The custom script uses different environment variable names (`TESTOMAT_API_KEY` vs `TESTOMATIO_API_KEY`).

## Environment Variables

### For Testomat CLI (Recommended)
- `TESTOMATIO_API_KEY` - Your Testomat API key
- `TESTOMATIO_PROJECT_ID` - Your Testomat project ID

### For Custom Script (Alternative)
- `TESTOMAT_API_KEY` - Your Testomat API key
- `TESTOMAT_PROJECT_ID` - Your Testomat project ID

## Configuration Options

The `testomat.yml` file contains various configuration options:

- **Framework**: Set to `cucumber` for Gherkin-based tests
- **Language**: Set to `rust` for Rust-based tests
- **Test Paths**: Configured to scan feature files in the qa-tools subdirectories
- **Labels**: Pre-configured labels for organizing tests
- **Report Settings**: Configured to look for XML reports in the reports directory

## Test Report Format

The script expects XML test reports in JUnit format, which is what your current test setup generates. The reports should be located in `../reports/` (relative to this directory).

## Troubleshooting

### Common Issues

1. **"API key not set" error**: Make sure you've set the correct environment variables
   - For CLI: `TESTOMATIO_API_KEY`
   - For script: `TESTOMAT_API_KEY`

2. **"Project ID not set" error**: Make sure you've set the correct environment variables
   - For CLI: `TESTOMATIO_PROJECT_ID`
   - For script: `TESTOMAT_PROJECT_ID`

3. **"Report file not found" error**: Check that the report file exists in the expected location

4. **"npm command not found"**: Make sure Node.js is installed and nvm is properly configured

5. **"nvm command not found"**: Source your shell configuration or install nvm

### Debug Mode

To see more detailed output with the Testomat CLI, you can add debug logging:

```bash
export DEBUG=* && npx @testomatio/reporter xml ../reports/tx_dispatcher.xml
```

## Integration with CI/CD

For continuous integration, you can:

1. Set the environment variables as secrets in your CI system
2. Run the CLI command after your tests complete
3. Use the CLI's exit code to determine if the report was sent successfully

Example GitHub Actions step:

```yaml
- name: Send test report to Testomat
  env:
    TESTOMATIO_API_KEY: ${{ secrets.TESTOMATIO_API_KEY }}
    TESTOMATIO_PROJECT_ID: ${{ secrets.TESTOMATIO_PROJECT_ID }}
  run: |
    cd qa-tools/testomat
    export NVM_DIR="$HOME/.nvm"
    [ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"
    nvm use v22.13.0
    npx @testomatio/reporter xml ../reports/tx_dispatcher.xml
```

## Success Indicators

When the report is successfully uploaded, you should see:
- `[TESTOMATIO] 📊 Report created. Report ID: [ID]`
- `[TESTOMATIO] 📊 Report Saved. Report URL: https://app.testomat.io/projects/[PROJECT]/runs/[ID]/report`

The report will be available in your Testomat.io dashboard under your project. 