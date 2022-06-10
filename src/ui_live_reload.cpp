// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#include <QFileSystemWatcher>
#include <QQmlApplicationEngine>
#include <QTimer>
#include <QUrl>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QDirIterator>

void init_live_reload(QQmlApplicationEngine *engine, const QString &path) {
    QFileSystemWatcher *w = new QFileSystemWatcher();
    QDirIterator it(path, QDirIterator::Subdirectories);
    while (it.hasNext()) {
        it.next();
        auto i = it.fileInfo();
        if (i.isFile())
            w->addPath(i.absoluteFilePath());
    }

    QUrl mainPath = QUrl::fromLocalFile(path + "/App.qml");

    QObject::connect(w, &QFileSystemWatcher::fileChanged, [=](const QString &file) {
        QTimer::singleShot(50, [=] {
            static QQuickItem *previousItem = nullptr;
            auto wnd = qobject_cast<QQuickWindow *>(engine->rootObjects().first());
            w->addPath(file);

            auto children = wnd->contentItem()->childItems();
            if (!children.isEmpty()) {
                auto itm = children.first();
                if (itm->objectName() == "App") {
                    itm->setParentItem(nullptr);
                    if (itm == previousItem) previousItem = nullptr;
                    delete itm;
                }
            }

            if (previousItem) {
                auto toDelete = previousItem;
                QTimer::singleShot(5000, [=] {
                    toDelete->setParentItem(nullptr);
                    delete toDelete;
                });
            }
            engine->clearComponentCache();

            QQmlComponent component(engine, mainPath, wnd);
            previousItem = qobject_cast<QQuickItem *>(component.create());
            if (previousItem) {
                previousItem->setObjectName("App");
                previousItem->setParentItem(wnd->contentItem());
            }
        });
    });
}
