import subprocess
import sys
from pathlib import Path

def getFromFile(fileName):
    with open(fileName, 'r') as file:
        content = file.read()
        lines_list = content.splitlines()
    return lines_list

def getDeletedApps(apps, installedApps):
    appsToInstallSet = set(apps)
    appsToDeleteList = []
    for app in installedApps:
        if not app in appsToInstallSet:
            appsToDeleteList.append(app)
    return appsToDeleteList

def getAddedApps(apps, installedApps):
    appsInstalledSet = set(installedApps)
    appsToInstallList = []
    for app in apps:
        if not app in appsInstalledSet:
            appsToInstallList.append(app)
    return appsToInstallList

def uninstallApps(deletedApps):
    for app in deletedApps:
        if pkg_manager == 'pacman':
            commandToUninstall = 'pacman -R ' + app
        else:
            commandToUninstall = pkg_manager + " remove -y " + app
        subprocess.run(commandToUninstall, shell=True)

def deleteUnneededFiles():
    if pkg_manager == 'apt':
        commandToDelete = 'apt autoremove --purge -y;'
        commandToDelete = 'apt autoclean'
    elif pkg_manager == 'pacman':
        commandToDelete = 'pacman -Rns $(pacman -Qdtq)'
    else:
        commandToDelete = pkg_manager + ' autoremove -y'
    subprocess.run(commandToDelete, shell=True)

def update():
    if pkg_manager == 'apt':
        commandToUpdate = 'apt update -y;'
        commandToUpdate += 'apt upgrade -y'
    elif pkg_manager == 'pacman':
        commandToUpdate = 'pacman -Syu'
    elif pkg_manager == 'zypper':
        commandToUpdate = 'zypper ref -y'
        commandToUpdate += 'zypper up -y'
    else:
        commandToUpdate = pkg_manager + ' upgrade -y'
    subprocess.run(commandToUpdate, shell=True)

def installApps(addedApps):
    update()
    for app in addedApps:
        if pkg_manager == 'pacman':
            commandToInstall = 'pacman -S app'
        else:
            commandToInstall = pkg_manager + ' install -y ' + app
        subprocess.run(commandToInstall, shell=True)


def writeToFile(apps):
    with open("installedApps.txt", 'w') as file:
        file.write('\n'.join(apps) + '\n') 

installedAppsFile = Path("installedApps.txt")
pkg_manager = sys.argv[1]
apps = [

]
if not installedAppsFile.exists():
    addedApps = apps
else:
    installedApps = getFromFile("installedApps.txt")
    deletedApps = getDeletedApps(apps, installedApps)
    addedApps = getAddedApps(apps, installedApps) # These two could really have been the same method, but inverted.
    uninstallApps(deletedApps)
    deleteUnneededFiles()
installApps(addedApps)
writeToFile(apps)

