# Makefile for Raven package manager
# Default installation paths
PREFIX ?= /usr
BINDIR = $(PREFIX)/bin
ETCDIR = /etc
VARDIR = /var/raven

# Installation target
install:
    # Install raven binary
    install -v -m 755 -o root -g root raven $(BINDIR)/raven || { echo "Failed to install raven binary"; exit 1; }
    
    # Install configuration file
    install -v -m 644 -o root -g root raven.conf $(ETCDIR)/raven.conf || { echo "Failed to install config file"; exit 1; }
    
    # Create and configure var directory if it doesn't exist
    if [ ! -d $(VARDIR) ]; then \
        mkdir -v -m 755 $(VARDIR) || { echo "Failed to create $(VARDIR)"; exit 1; }; \
        chown -v root:root $(VARDIR) || { echo "Failed to set ownership for $(VARDIR)"; exit 1; }; \
    fi

# Uninstall target (removes only binary and config)
uninstall:
    rm -vf $(BINDIR)/raven || { echo "Failed to remove raven binary"; exit 1; }
    rm -vf $(ETCDIR)/raven.conf || { echo "Failed to remove config file"; exit 1; }

# Complete uninstall target (removes everything including var directory)
uninstall_all:
    rm -vf $(BINDIR)/raven || { echo "Failed to remove raven binary"; exit 1; }
    rm -vf $(ETCDIR)/raven.conf || { echo "Failed to remove config file"; exit 1; }
    rm -rfv $(VARDIR) || { echo "Failed to remove $(VARDIR)"; exit 1; }

.PHONY: install uninstall uninstall_all
