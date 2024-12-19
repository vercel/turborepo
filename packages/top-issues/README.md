# `@turbo/top-issues`

This is an internal package that is used by a GitHub Actions Workflow to post
top issues in `vercel/turbo` to Slack.

The code here gets the top issues and writes them to a file. The GitHub Action
workflow will then take that file and post it to Slack with a marketplace
action.
