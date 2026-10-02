// SPDX-License-Identifier: GPL-2.0-or-later
// Settings dialog: Tools → SIP Call-In… (DESIGN.md §5.2).
// Settings travel as JSON (the same keys as config.json), so the C
// interface is one string in and one string out. UI thread only.

#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QDialog>
#include <QtWidgets/QDialogButtonBox>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QLineEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QSpinBox>
#include <QtWidgets/QVBoxLayout>

#include <memory>

extern "C" {
typedef void (*sp_settings_save_fn)(const char *json);
void sp_settings_open(void *parent, const char *json, sp_settings_save_fn on_save);
}

namespace {

int defaultPort(const QString &transport)
{
	return transport == "tls" ? 5061 : 5060;
}

QComboBox *combo(QWidget *parent, const QList<QPair<QString, QString>> &items, const QString &current)
{
	auto *c = new QComboBox(parent);
	for (const auto &item : items)
		c->addItem(item.first, item.second);
	int i = c->findData(current);
	c->setCurrentIndex(i < 0 ? 0 : i);
	return c;
}

const char *kTreasureHint = "Copy these from <b>Admin → Call-In → Studio Softphone</b> in Treasure Stream.";
const char *kGenericHint = "Use the SIP account details from your phone system or VoIP provider. "
			   "The plugin only receives calls.";

} // namespace

void sp_settings_open(void *parent, const char *json, sp_settings_save_fn on_save)
{
	QJsonObject in = QJsonDocument::fromJson(QByteArray(json)).object();

	auto *dlg = new QDialog(static_cast<QWidget *>(parent));
	dlg->setAttribute(Qt::WA_DeleteOnClose);
	dlg->setWindowTitle("SIP Call-In");
	auto *layout = new QVBoxLayout(dlg);

	// Wrapped labels inside a QFormLayout row get too little height, so the
	// hint sits between two forms.
	auto *top = new QFormLayout();
	auto *preset = combo(dlg, {{"Treasure Stream", "treasure_stream"}, {"Other SIP server", "generic"}},
			     in.value("preset").toString("generic"));
	top->addRow("Server type", preset);
	layout->addLayout(top);

	auto *hint = new QLabel(dlg);
	hint->setWordWrap(true);
	layout->addWidget(hint);

	auto *form = new QFormLayout();
	auto *enabled = new QCheckBox("Connect", dlg);
	enabled->setChecked(in.value("enabled").toBool(true));
	form->addRow(enabled);

	auto *server = new QLineEdit(in.value("server").toString(), dlg);
	server->setPlaceholderText("sip.example.com");
	form->addRow("Server", server);

	QString transport0 = in.value("transport").toString("udp");
	auto *transport = combo(dlg, {{"UDP", "udp"}, {"TCP", "tcp"}, {"TLS (encrypted)", "tls"}}, transport0);
	form->addRow("Transport", transport);

	auto *port = new QSpinBox(dlg);
	port->setRange(1, 65535);
	port->setValue(in.value("port").toInt(defaultPort(transport0)));
	form->addRow("Port", port);

	auto *username = new QLineEdit(in.value("username").toString(), dlg);
	username->setPlaceholderText("extension or account name");
	form->addRow("Username", username);

	auto *password = new QLineEdit(in.value("password").toString(), dlg);
	password->setEchoMode(QLineEdit::PasswordEchoOnEdit);
	form->addRow("Password", password);

	auto *track = new QComboBox(dlg);
	for (int t = 1; t <= 6; ++t)
		track->addItem(QString("Track %1").arg(t), t);
	track->setCurrentIndex(qBound(1, in.value("return_track").toInt(2), 6) - 1);
	form->addRow("Caller hears", track);

	auto *autoAnswer = new QCheckBox("Answer calls automatically", dlg);
	autoAnswer->setChecked(in.value("auto_answer").toBool(true));
	form->addRow(autoAnswer);

	auto *chime = new QCheckBox("Quiet ring chime on this computer (never on air)", dlg);
	chime->setChecked(in.value("ring_chime").toBool(true));
	form->addRow(chime);

	auto *replaced = new QCheckBox("Warn when another device signs in with this account", dlg);
	replaced->setChecked(in.value("replaced_detection").toBool(true));
	form->addRow(replaced);
	layout->addLayout(form);

	// --- Advanced ---------------------------------------------------------
	auto *advToggle = new QPushButton("Advanced ▸", dlg);
	advToggle->setFlat(true);
	advToggle->setStyleSheet("text-align: left;");
	layout->addWidget(advToggle);

	auto *advanced = new QWidget(dlg);
	auto *adv = new QFormLayout(advanced);
	adv->setContentsMargins(0, 0, 0, 0);

	auto *authUser = new QLineEdit(in.value("auth_username").toString(), advanced);
	authUser->setPlaceholderText("same as username");
	adv->addRow("Auth username", authUser);

	auto *domain = new QLineEdit(in.value("domain").toString(), advanced);
	domain->setPlaceholderText("same as server");
	adv->addRow("Domain", domain);

	auto *proxy = new QLineEdit(in.value("outbound_proxy").toString(), advanced);
	proxy->setPlaceholderText("none (host or host:port)");
	adv->addRow("Outbound proxy", proxy);

	auto *srtp = combo(advanced, {{"Off", "off"}, {"Optional", "optional"}, {"Required", "required"}},
			   in.value("srtp").toString("off"));
	adv->addRow("Encrypted media (SRTP)", srtp);

	auto *stun = new QLineEdit(in.value("stun_server").toString(), advanced);
	stun->setPlaceholderText("none (host or host:port)");
	adv->addRow("STUN server", stun);

	auto *verify = new QCheckBox("Verify the server's TLS certificate (recommended)", advanced);
	verify->setChecked(in.value("verify_tls").toBool(true));
	adv->addRow(verify);
	layout->addWidget(advanced);

	auto showAdvanced = [=](bool show) {
		advanced->setVisible(show);
		advToggle->setText(show ? "Advanced ▾" : "Advanced ▸");
	};
	bool advancedInUse = !authUser->text().isEmpty() || !domain->text().isEmpty() || !proxy->text().isEmpty() ||
			     !stun->text().isEmpty() || srtp->currentData().toString() != "off" ||
			     !verify->isChecked();
	showAdvanced(advancedInUse);
	QObject::connect(advToggle, &QPushButton::clicked, dlg, [=] { showAdvanced(!advanced->isVisible()); });

	auto *trackHint =
		new QLabel("Put everything the caller should hear (mics, music) on this track, and keep "
			   "<i>Call-In Caller</i> off it: Audio Mixer → ⋮ → Advanced Audio Properties.",
			   dlg);
	trackHint->setWordWrap(true);
	trackHint->setEnabled(false);
	layout->addWidget(trackHint);

	// --- Behaviour ----------------------------------------------------------
	// Changing the transport moves the port along if it was still the old
	// transport's default.
	auto lastTransport = std::make_shared<QString>(transport0);
	QObject::connect(transport, &QComboBox::currentIndexChanged, dlg, [=] {
		QString t = transport->currentData().toString();
		if (port->value() == defaultPort(*lastTransport))
			port->setValue(defaultPort(t));
		*lastTransport = t;
		verify->setEnabled(t == "tls");
	});
	verify->setEnabled(transport0 == "tls");

	auto applyHint = [=] {
		hint->setText(preset->currentData().toString() == "treasure_stream" ? kTreasureHint : kGenericHint);
	};
	applyHint();
	// Picking a preset fills in its usual connection settings.
	QObject::connect(preset, &QComboBox::currentIndexChanged, dlg, [=] {
		applyHint();
		bool ts = preset->currentData().toString() == "treasure_stream";
		transport->setCurrentIndex(transport->findData(ts ? "tls" : "udp"));
		if (ts && username->text().isEmpty())
			username->setText("101");
	});

	auto *buttons = new QDialogButtonBox(QDialogButtonBox::Save | QDialogButtonBox::Cancel, dlg);
	layout->addWidget(buttons);
	QObject::connect(buttons, &QDialogButtonBox::rejected, dlg, &QDialog::reject);
	QObject::connect(buttons, &QDialogButtonBox::accepted, dlg, [=] {
		QJsonObject out;
		out["preset"] = preset->currentData().toString();
		out["enabled"] = enabled->isChecked();
		out["server"] = server->text().trimmed();
		out["transport"] = transport->currentData().toString();
		out["port"] = port->value();
		out["username"] = username->text().trimmed();
		out["password"] = password->text();
		out["return_track"] = track->currentData().toInt();
		out["auto_answer"] = autoAnswer->isChecked();
		out["ring_chime"] = chime->isChecked();
		out["replaced_detection"] = replaced->isChecked();
		out["auth_username"] = authUser->text().trimmed();
		out["domain"] = domain->text().trimmed();
		out["outbound_proxy"] = proxy->text().trimmed();
		out["srtp"] = srtp->currentData().toString();
		out["stun_server"] = stun->text().trimmed();
		out["verify_tls"] = verify->isChecked();
		QByteArray bytes = QJsonDocument(out).toJson(QJsonDocument::Compact);
		on_save(bytes.constData());
		dlg->accept();
	});

	dlg->setMinimumWidth(460);
	dlg->open();
}
