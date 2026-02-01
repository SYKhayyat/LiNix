#!/usr/bin/env bash
if command -v apt &> /dev/null; then
	pkg_manager="apt"
elif command -v dnf &> /dev/null; then
	pkg_manager="dnf"
elif command -v yum &> /dev/null; then
    pkg_manager="yum"
elif command -v pacman &> /dev/null; then
    pacman -Syu
	pacman -S python	
elif command -v zypper &> /dev/null; then
    zypper ref 
	zypper install python3
else
    echo "Could not identify package manager"
fi
$pkg_manager update
$pkg_manager install python3
chmod +x declareApps.py
python3 declareApps.py $pkg_manager
