UPDATE audit_logs AS a SET actor_label=u.email
FROM users AS u WHERE a.actor_id=u.id AND a.actor_label IS NULL;
