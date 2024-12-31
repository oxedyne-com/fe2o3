# Git Workflow Cheat Sheet

## 1. Branch Management

### Check Current Status
```bash
git status                  # Show changed files
git branch                  # List branches (* shows current)
git branch --show-current  # Just show current branch name
```

### Creating & Switching Branches
```bash
git checkout -b new-branch     # Create and switch to new branch
git checkout existing-branch   # Switch to existing branch
```

### Stashing Changes
```bash
git stash                     # Save current changes
git stash list               # Show all stashes
git stash pop                # Apply and remove most recent stash
git stash apply stash@{1}    # Apply specific stash without removing
git stash drop stash@{1}     # Remove specific stash
```

## 2. Working with Changes

### Basic Workflow
```bash
git status                    # Check what's changed
git add specific-file.txt     # Stage specific file
git add .                     # Stage all changes
git commit -m "Message"       # Commit with message
```

### Selective Committing
```bash
git add -p                    # Interactively choose parts to stage
git restore --staged file.txt # Unstage a file
git diff                      # Show unstaged changes
git diff --staged            # Show staged changes
```

### Fixing Mistakes
```bash
git reset --soft HEAD^        # Undo last commit, keep changes staged
git reset --hard HEAD^        # Undo last commit, discard changes
git restore file.txt         # Discard changes in file
```

## 3. Feature Branch Workflow

### Start New Feature
```bash
git checkout main
git pull                      # Get latest main
git checkout -b feature-name  # Create feature branch
```

### Work on Feature
```bash
# Make changes
git add .
git commit -m "Feature progress"
git push -u origin feature-name  # First push
git push                        # Subsequent pushes
```

### Finish Feature
```bash
git checkout main
git pull                     # Get any new changes
git merge feature-name       # Merge feature into main
git push                     # Push merged changes
git branch -d feature-name   # Delete local branch
git push origin -d feature-name  # Delete remote branch
```

## 4. Common Scenarios

### Save Work in Progress
```bash
git diff > my_changes.patch   # Backup changes to file
git stash                     # Or use stash
```

### Handle Merge Conflicts
```bash
git status                    # See conflicted files
# Edit files to resolve conflicts
git add resolved-file.txt    
git commit                    # Complete the merge
```

### Update Feature Branch with Main
```bash
git checkout feature-branch
git merge main               # Get changes from main
```

## 5. Useful Commands

### View History
```bash
git log                      # Show commit history
git log --oneline           # Compact history view
git show commit-hash        # Show specific commit details
```

### Remote Operations
```bash
git remote -v               # Show remote repositories
git fetch                   # Get remote changes without merging
git pull                    # Get and merge remote changes
```

### Configuration
```bash
git config --global user.name "Your Name"
git config --global user.email "your.email@example.com"
```

## 6. Tips

- Always check which branch you're on before making changes
- Commit frequently with clear messages
- Pull from main regularly to stay up to date
- Use feature branches for new work
- Backup important changes before risky operations
- Add generated files and build artifacts to .gitignore
