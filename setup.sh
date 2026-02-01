#!/usr/bin/env bash
is_applicable=1
if command -v apt &> /dev/null; then
    pkg_manager="apt"
elif command -v dnf &> /dev/null; then
    pkg_manager="dnf"
elif command -v yum &> /dev/null; then
    pkg_manager="yum"
elif command -v pacman &> /dev/null; then
    if ! command -v python; then
        pacman -Syu
        pacman -S python
    fi	
elif command -v zypper &> /dev/null; then
    if ! command -v python3; then
        zypper ref 
	zypper install python3
    fi
    else
        echo "Could not identify package manager"
        is_applicable=0
fi
if ((is_applicable == 1)); then
    if ! command -v python3; then
        $pkg_manager update
        $pkg_manager install python3
    fi
chmod +x declareApps.py
    python3 declareApps.py $pkg_manager
fi
